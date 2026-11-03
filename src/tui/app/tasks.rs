//! Advisory task accounting stays outside the signer. Jobs own captured key/chain context;
//! late read results never repopulate a screen after cancellation or a context change.
use super::*;
use crate::tui::{
    actions::ActionId,
    form::{Edit, Field, Input, Kind},
};
use crate::wallet::tasks::{self, TaskCreate, TaskReport, TaskSummary};
use std::path::Path;
use tokio::sync::{mpsc::UnboundedSender, watch};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Closed,
    List,
    CreateId,
    Allocations,
    ConfirmCreate,
    ReportId,
    Loading,
    Report,
    Error,
}
#[derive(Default)]
pub struct Tasks {
    pub stage: Stage,
    pub rows: Vec<TaskSummary>,
    pub selected: usize,
    pub scroll: usize,
    pub field: Option<Field>,
    pub notice: Option<Outside>,
    pub report: Option<TaskReport>,
    pub draft: Option<TaskCreate>,
    pub creating: bool,
    pub offline: bool,
    pub reveal: bool,
    pub id: String,
    counter: u64,
    active: Option<u64>,
    context: Option<(String, &'static str, String)>,
    jobs: BTreeMap<u64, TaskJob>,
    cancel: Option<watch::Sender<bool>>,
}
#[derive(Debug)]
pub enum TaskMsg {
    List(Result<Vec<TaskSummary>>),
    Created(Result<TaskSummary>),
    Report(Result<TaskReport>),
}
pub enum Request {
    List,
    Create {
        net: Net,
        input: TaskCreate,
    },
    Report {
        id: String,
        nets: Vec<Net>,
        offline: bool,
        reveal: bool,
    },
}
pub struct TaskJob {
    request: Request,
    cancel: watch::Receiver<bool>,
}
async fn metadata<T: Send + 'static>(
    read: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    static SLOTS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    let permit = SLOTS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| WalletError::validation("task metadata worker unavailable"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        read()
    })
    .await
    .map_err(|_| WalletError::validation("task metadata worker failed"))?
}

impl TaskJob {
    pub async fn run(self, id: u64, home: &Path, tx: &UnboundedSender<Msg>) {
        let mut cancel = self.cancel;
        let creating = matches!(self.request, Request::Create { .. });
        let read = async {
            match self.request {
                Request::List => {
                    let home = home.to_path_buf();
                    TaskMsg::List(metadata(move || tasks::list(&home)).await)
                }
                Request::Create { net, input } => {
                    TaskMsg::Created(tasks::create(home, &net, &input).await)
                }
                Request::Report {
                    id,
                    nets,
                    offline,
                    reveal,
                } => {
                    let result = async {
                        let (info_home, info_id) = (home.to_path_buf(), id.clone());
                        let task = metadata(move || tasks::info(&info_home, &info_id)).await?;
                        let net = nets
                            .iter()
                            .find(|n| n.chain.key == task.chain)
                            .ok_or_else(|| WalletError::validation("task chain is unavailable"))?;
                        tasks::report(home, &id, net, offline, reveal).await
                    }
                    .await;
                    TaskMsg::Report(result)
                }
            }
        };
        // A submitted atomic local create may finish after navigation. Reads are cancellable.
        let event = if creating {
            read.await
        } else {
            tokio::select! { biased; _ = cancel.changed() => return, event = read => event }
        };
        let _ = tx.send(Msg::Tasks(id, event));
    }
}
impl App {
    pub fn tasks(&self) -> &Tasks {
        &self.tasks
    }
    pub fn tasks_visible(&self) -> bool {
        self.tab() == Tab::Tools && self.tasks.stage != Stage::Closed
    }
    pub fn take_task_job(&mut self, id: u64) -> Option<TaskJob> {
        self.tasks.jobs.remove(&id)
    }
    fn task_context(&self) -> (String, &'static str, String) {
        (
            self.open_key.clone().unwrap_or_default(),
            self.chain.key,
            self.net().rpc_url.clone(),
        )
    }
    pub(super) fn close_tasks(&mut self) {
        if let Some(cancel) = self.tasks.cancel.take() {
            let _ = cancel.send(true);
        }
        let counter = self.tasks.counter;
        self.tasks = Tasks {
            counter,
            ..Tasks::default()
        };
    }
    fn task_job(&mut self, request: Request) -> Vec<Effect> {
        if let Some(cancel) = self.tasks.cancel.take() {
            let _ = cancel.send(true);
        }
        self.tasks.creating = matches!(request, Request::Create { .. });
        self.tasks.jobs.clear();
        self.tasks.counter += 1;
        let id = self.tasks.counter;
        let (cancel, receiver) = watch::channel(false);
        self.tasks.cancel = Some(cancel);
        self.tasks.active = Some(id);
        self.tasks.jobs.insert(
            id,
            TaskJob {
                request,
                cancel: receiver,
            },
        );
        self.tasks.stage = Stage::Loading;
        self.tasks.notice = None;
        self.tasks.field = None;
        self.tasks.report = None;
        vec![Effect::Tasks(id)]
    }
    pub(super) fn start_tasks(&mut self, action: ActionId) -> Vec<Effect> {
        self.close_tasks();
        self.tools_reset();
        self.nav.tab = Tab::Tools;
        self.nav.popup = None;
        self.tasks.context = Some(self.task_context());
        match action {
            ActionId::TaskCreate => {
                let Some(key) = self.open_key.clone() else {
                    self.tasks.stage = Stage::Error;
                    self.tasks.notice = Some(Outside::detail(
                        "Open a key first; task creation never chooses the global active key.",
                    ));
                    return vec![];
                };
                self.tasks.draft = Some(TaskCreate {
                    id: String::new(),
                    key,
                    allocations: BTreeMap::new(),
                });
                self.tasks.stage = Stage::CreateId;
                self.tasks.field = Some(Field::new(Kind::TaskId));
                vec![]
            }
            ActionId::TaskReport => {
                self.tasks.stage = Stage::ReportId;
                self.tasks.field = Some(Field::new(Kind::TaskId));
                vec![]
            }
            _ => self.task_job(Request::List),
        }
    }
    fn read_task_report(&mut self) -> Vec<Effect> {
        self.tasks.scroll = 0;
        self.task_job(Request::Report {
            id: self.tasks.id.clone(),
            nets: self.nets(),
            offline: self.tasks.offline,
            reveal: self.tasks.reveal,
        })
    }
    pub(super) fn tasks_update(&mut self, msg: Msg) -> std::ops::ControlFlow<Msg, Vec<Effect>> {
        use std::ops::ControlFlow::{Break, Continue};
        if self.tasks.creating {
            match &msg {
                Msg::Key(Key::Char('q') | Key::Interrupt) => {
                    self.quit = true;
                    return Continue(vec![]);
                }
                Msg::Key(Key::Char('L')) => return Break(msg),
                Msg::Key(_) | Msg::Paste(_) => {
                    self.tasks.notice = Some(Outside::detail(
                        "Saving the submitted task; wait for its result. q waits then exits.",
                    ));
                    return Continue(vec![]);
                }
                _ => {}
            }
        }
        if !self.tasks.creating
            && self.tasks.stage != Stage::Closed
            && self.tasks.context.as_ref() != Some(&self.task_context())
        {
            self.close_tasks();
        }
        match msg {
            Msg::Tasks(id, event) => {
                if self.tasks.active != Some(id) {
                    return Continue(vec![]);
                }
                self.tasks.active = None;
                self.tasks.creating = false;
                self.tasks.cancel = None;
                let result = match event {
                    TaskMsg::List(Ok(rows)) => {
                        self.tasks.rows = rows;
                        self.tasks.selected = 0;
                        self.tasks.stage = Stage::List;
                        Ok(())
                    }
                    TaskMsg::Created(Ok(task)) => {
                        if self.quit {
                            self.tasks.rows = vec![task];
                            self.tasks.stage = Stage::List;
                            return Continue(vec![]);
                        }
                        self.tasks.id = task.id;
                        self.tasks.offline = true;
                        return Continue(self.read_task_report());
                    }
                    TaskMsg::Report(Ok(report)) => {
                        self.tasks.report = Some(report);
                        self.tasks.stage = Stage::Report;
                        Ok(())
                    }
                    TaskMsg::List(Err(e)) | TaskMsg::Created(Err(e)) | TaskMsg::Report(Err(e)) => {
                        Err(e)
                    }
                };
                if let Err(e) = result {
                    if self.quit {
                        self.quit_error.get_or_insert_with(|| e.clone());
                    }
                    self.tasks.notice = Some(Outside::error(&e));
                    self.tasks.stage = Stage::Error;
                }
                Continue(vec![])
            }
            Msg::Key(key)
                if self.tasks_visible()
                    && self.password == PasswordState::Idle
                    && self.nav.popup.is_none() =>
            {
                if self.tasks.field.is_some() {
                    return Continue(self.task_input(super::navigation::input(key)));
                }
                if super::navigation::switches_tab(&super::navigation::input(key))
                    || matches!(
                        key,
                        Key::Char('q' | 'L' | 'c' | 'u' | 'V' | '?' | ':') | Key::Interrupt
                    )
                {
                    self.close_tasks();
                    return Break(Msg::Key(key));
                }
                let work = match (self.tasks.stage, key) {
                    (_, Key::Esc) => {
                        self.close_tasks();
                        vec![]
                    }
                    (Stage::List, Key::Char('n')) => self.start_tasks(ActionId::TaskCreate),
                    (Stage::List, Key::Char('r')) | (Stage::Error, Key::Enter) => {
                        self.start_tasks(ActionId::TaskList)
                    }
                    (Stage::List, Key::Up | Key::Char('k')) => {
                        self.tasks.selected = self.tasks.selected.saturating_sub(1);
                        vec![]
                    }
                    (Stage::List, Key::Down | Key::Char('j')) => {
                        self.tasks.selected =
                            (self.tasks.selected + 1).min(self.tasks.rows.len().saturating_sub(1));
                        vec![]
                    }
                    (Stage::List, Key::Enter) => {
                        if let Some(row) = self.tasks.rows.get(self.tasks.selected) {
                            self.tasks.id = row.id.clone();
                            self.tasks.offline = false;
                            self.tasks.reveal = false;
                            self.read_task_report()
                        } else {
                            vec![]
                        }
                    }
                    (Stage::ConfirmCreate, Key::Enter) => {
                        if let Some(input) = self.tasks.draft.take() {
                            self.task_job(Request::Create {
                                net: self.net().clone(),
                                input,
                            })
                        } else {
                            vec![]
                        }
                    }
                    (Stage::Report, Key::Char('r')) => {
                        self.tasks.offline = false;
                        self.read_task_report()
                    }
                    (Stage::Report, Key::Char('o')) => {
                        self.tasks.offline = !self.tasks.offline;
                        self.read_task_report()
                    }
                    (Stage::Report, Key::Char('a')) => {
                        self.tasks.reveal = !self.tasks.reveal;
                        self.read_task_report()
                    }
                    (Stage::Report | Stage::ConfirmCreate, Key::Down | Key::Char('j')) => {
                        self.tasks.scroll = self.tasks.scroll.saturating_add(1);
                        vec![]
                    }
                    (Stage::Report | Stage::ConfirmCreate, Key::Up | Key::Char('k')) => {
                        self.tasks.scroll = self.tasks.scroll.saturating_sub(1);
                        vec![]
                    }
                    (Stage::Report | Stage::ConfirmCreate, Key::Home) => {
                        self.tasks.scroll = 0;
                        vec![]
                    }
                    _ => vec![],
                };
                Continue(work)
            }
            Msg::Paste(text)
                if self.tasks_visible()
                    && self.password == PasswordState::Idle
                    && self.nav.popup.is_none() =>
            {
                Continue(self.task_input(Input::Paste(&text.0)))
            }
            other => Break(other),
        }
    }
    fn task_input(&mut self, input: Input<'_>) -> Vec<Effect> {
        let Some(mut field) = self.tasks.field.take() else {
            return vec![];
        };
        self.tasks.notice = None;
        match field.edit(input) {
            Edit::Submit => match self.tasks.stage {
                Stage::CreateId => {
                    self.tasks.draft.as_mut().unwrap().id = field.value().into();
                    self.tasks.stage = Stage::Allocations;
                    self.tasks.field = Some(Field::new(Kind::TaskAllocations));
                    return vec![];
                }
                Stage::Allocations => {
                    let values: Vec<_> = if field.value().is_empty() {
                        vec![]
                    } else {
                        field.value().split(',').map(str::to_string).collect()
                    };
                    match tasks::parse_allocations(self.chain, &values) {
                        Ok(allocations) => {
                            self.tasks.draft.as_mut().unwrap().allocations = allocations;
                            self.tasks.stage = Stage::ConfirmCreate;
                            return vec![];
                        }
                        Err(e) => self.tasks.notice = Some(Outside::error(&e)),
                    }
                }
                Stage::ReportId => {
                    self.tasks.id = field.value().into();
                    return self.read_task_report();
                }
                _ => {}
            },
            Edit::Cancel => {
                self.close_tasks();
                return vec![];
            }
            Edit::Interrupt => {
                self.close_tasks();
                self.quit = true;
                return vec![];
            }
            Edit::Rejected(reason) => self.tasks.notice = Some(Outside::detail(reason)),
            _ => {}
        }
        self.tasks.field = Some(field);
        vec![]
    }
    pub fn tasks_hint(&self) -> &'static str {
        match self.tasks.stage {
            Stage::List => "arrows select  Enter report  n create  r refresh  Esc tools",
            Stage::CreateId | Stage::Allocations | Stage::ReportId => {
                "Enter continue  arrows edit  Esc cancel  Ctrl-C quit"
            }
            Stage::ConfirmCreate => {
                "j/k scroll  Enter create local task  Esc cancel; no signature or budget"
            }
            Stage::Loading if self.tasks.creating => {
                "saving local task... q waits for completion; no cancel after submission"
            }
            Stage::Loading => "reading... Esc cancels; stalled filesystem workers remain bounded",
            Stage::Report => {
                "j/k scroll  Home top  r online refresh  o offline/online  a reveal/hide  Esc tools"
            }
            _ => "Enter task list  Esc tools",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chains,
        tui::{term::FilteredBackend, ui},
    };
    use ratatui::{Terminal, backend::TestBackend};

    fn app(chain: &str) -> App {
        let net = Net {
            chain: chains::find(chain).unwrap(),
            rpc_url: "http://127.0.0.1:9".into(),
        };
        let mut app = App::new(&net, Duration::from_secs(60));
        app.open_key = Some("selected-key".into());
        app
    }
    fn type_text(app: &mut App, value: &str) {
        for c in value.chars() {
            app.update(Msg::Key(Key::Char(c)));
        }
    }
    fn summary() -> TaskSummary {
        TaskSummary {
            id: "fixture".into(),
            chain: "ethereum".into(),
            owner: "private-owner-should-not-be-shown".into(),
            created_at: 1,
            intent_count: 0,
        }
    }
    fn report() -> TaskReport {
        TaskReport {
            task_id: "fixture".into(),
            chain: "ethereum".into(),
            observed_at: 123,
            observation_started_at: 120,
            baseline_observed_at: 1,
            baseline_source: "http://127.0.0.1:9".into(),
            current_source: None,
            offline: true,
            owner: "[redacted]".into(),
            records: vec![],
            assets: vec![tasks::TaskAssetReport {
                asset: "native".into(),
                symbol: "ETH".into(),
                decimals: 18,
                allocation_raw: Some("9".repeat(78)),
                baseline_balance_raw: None,
                current_balance_raw: None,
                balance_change_raw: None,
                confirmed_intent_raw: "0".into(),
                known_fees_raw: "0".into(),
                remaining_allocation_raw: None,
                accounting_complete: false,
            }],
            warnings: vec!["unverified\x1b[2J\u{2588}".into()],
        }
    }
    fn screen(app: &App, size: (u16, u16)) -> String {
        let mut terminal =
            Terminal::new(FilteredBackend::new(TestBackend::new(size.0, size.1))).unwrap();
        let mut placed = vec![];
        terminal.draw(|f| placed = ui::draw(f, app)).unwrap();
        assert_eq!(terminal.backend().replaced(), 0);
        for p in placed {
            assert!(p.outside.text().len() <= p.outside.source().budget());
            assert!(p.area.right() <= size.0 && p.area.bottom() <= size.1);
        }
        let buffer = terminal.backend().inner().buffer();
        (0..size.1)
            .map(|y| {
                (0..size.0)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn successful_send_keeps_task_warning_visible_after_receipt_changes() {
        use super::super::transfer::{ReceiptState, TransferMsg};
        use crate::wallet::{SendTransferOutcome, send::SendOutput};
        for chain in ["ethereum", "tron"] {
            let mut app = app(chain);
            let id = app.begin_external_send();
            app.update(Msg::Transfer(
                id,
                TransferMsg::Progress {
                    hash: format!("0x{}", "1".repeat(64)),
                    nonce: 7,
                    journal: "/tmp/mock-journal".into(),
                },
            ));
            let work = app.update(Msg::Transfer(
                id,
                TransferMsg::Sent(Ok(SendTransferOutcome::Sent(SendOutput {
                    fields: serde_json::json!({"taskWarning": "submission update not confirmed"})
                        .as_object()
                        .unwrap()
                        .clone(),
                    duplicates_text: None,
                }))),
            ));
            let [Effect::Transfer(watch)] = work.as_slice() else {
                panic!("{work:?}")
            };
            let mut watch = *watch;
            for (attempt, receipt) in [
                Err(WalletError::rpc("receipt temporarily unavailable")),
                Ok(Some(serde_json::json!({"status":"mined", "success":true}))),
            ]
            .into_iter()
            .enumerate()
            {
                if attempt > 0 {
                    app.update(Msg::Key(Key::Char('w')));
                    assert!(
                        screen(&app, (80, 24))
                            .contains("Inspect task report; do not resend blindly.")
                    );
                    type_text(&mut app, "5m");
                    let work = app.update(Msg::Key(Key::Enter));
                    let [Effect::Transfer(next)] = work.as_slice() else {
                        panic!("{work:?}")
                    };
                    watch = *next;
                }
                app.update(Msg::Transfer(watch, TransferMsg::Receipt(receipt)));
                assert!(app.transfer.status.as_ref().unwrap().task_warning.is_some());
                for size in [(80, 24), (120, 40)] {
                    let shown = screen(&app, size);
                    assert!(
                        shown.contains("Task submission update not confirmed."),
                        "{shown}"
                    );
                    assert!(
                        shown.contains("Inspect task report; do not resend blindly."),
                        "{shown}"
                    );
                }
            }
            assert_eq!(
                app.transfer.status.as_ref().unwrap().state,
                ReceiptState::Success
            );
        }
    }

    #[test]
    fn create_captures_selected_key_and_chain_without_active_metadata_changes() {
        for chain in ["ethereum", "tron"] {
            let mut app = app(chain);
            assert!(app.start_tasks(ActionId::TaskCreate).is_empty());
            type_text(&mut app, "task-1");
            app.update(Msg::Key(Key::Enter));
            type_text(&mut app, "native=123");
            assert!(app.update(Msg::Key(Key::Enter)).is_empty());
            assert_eq!(app.tasks.stage, Stage::ConfirmCreate);
            for size in [(80, 24), (120, 40)] {
                let shown = screen(&app, size);
                assert!(shown.contains("selected-key"), "{shown}");
                assert!(shown.contains("native = 123 base units"));
            }
            let work = app.update(Msg::Key(Key::Enter));
            let [Effect::Tasks(id)] = work.as_slice() else {
                panic!("{work:?}")
            };
            let job = app.take_task_job(*id).unwrap();
            assert!(app.take_task_job(*id).is_none());
            match job.request {
                Request::Create { net, input } => {
                    assert_eq!(net.chain.key, chain);
                    assert_eq!(input.key, "selected-key");
                    assert_eq!(input.id, "task-1");
                    assert_eq!(input.allocations["native"], "123");
                }
                _ => panic!("not a create job"),
            }
        }
    }
    #[test]
    fn task_input_rejects_secrets_controls_and_invalid_allocation_without_submitting() {
        let mut app = app("ethereum");
        app.start_tasks(ActionId::TaskCreate);
        app.update(Msg::Paste(UserText(
            "a password with spaces".to_string().into(),
        )));
        assert_eq!(app.tasks.field.as_ref().unwrap().value(), "");
        type_text(&mut app, "safe");
        app.update(Msg::Key(Key::Enter));
        app.update(Msg::Paste(UserText("native=1\x1b[2J".to_string().into())));
        assert_eq!(app.tasks.field.as_ref().unwrap().value(), "");
        type_text(&mut app, "native=1,native=2");
        assert!(app.update(Msg::Key(Key::Enter)).is_empty());
        assert_eq!(app.tasks.stage, Stage::Allocations);
        assert!(app.tasks.notice.is_some());
    }
    #[test]
    fn cancelling_or_changing_context_drops_late_task_reads() {
        for change in [false, true] {
            let mut app = app("ethereum");
            let work = app.start_tasks(ActionId::TaskList);
            let [Effect::Tasks(id)] = work.as_slice() else {
                panic!()
            };
            let mut job = app.take_task_job(*id).unwrap();
            if change {
                app.open_key = Some("different-key".into());
                app.update(Msg::Tick);
            } else {
                app.update(Msg::Key(Key::Esc));
            }
            assert!(job.cancel.has_changed().unwrap_or(true));
            assert!(*job.cancel.borrow_and_update());
            app.update(Msg::Tasks(*id, TaskMsg::List(Ok(vec![summary()]))));
            assert_eq!(app.tasks.stage, Stage::Closed);
            assert!(app.tasks.rows.is_empty());
        }
    }
    #[test]
    fn list_hides_owner_and_report_survives_tick_with_redacted_default() {
        let mut app = app("ethereum");
        let work = app.start_tasks(ActionId::TaskList);
        let [Effect::Tasks(id)] = work.as_slice() else {
            panic!()
        };
        app.update(Msg::Tasks(*id, TaskMsg::List(Ok(vec![summary()]))));
        for size in [(80, 24), (120, 40)] {
            let shown = screen(&app, size);
            assert!(shown.contains("fixture"));
            assert!(!shown.contains("private-owner"));
        }
        let work = app.update(Msg::Key(Key::Enter));
        let [Effect::Tasks(id)] = work.as_slice() else {
            panic!()
        };
        let job = app.take_task_job(*id).unwrap();
        assert!(matches!(
            job.request,
            Request::Report {
                reveal: false,
                offline: false,
                ..
            }
        ));
        app.update(Msg::Tasks(*id, TaskMsg::Report(Ok(report()))));
        app.update(Msg::Tick);
        assert_eq!(app.tasks.stage, Stage::Report);
        for size in [(80, 24), (120, 40)] {
            let shown = screen(&app, size);
            assert!(shown.contains("observed 123"), "{shown}");
            assert!(shown.contains("local records only"), "{shown}");
            assert!(!shown.contains('\x1b') && !shown.contains('\u{2588}'));
            assert!(shown.contains("\\u{1b}"), "{shown}");
            // Exact 78-digit amount survives wrapping; no source-budget ellipsis.
            let joined = shown
                .lines()
                .map(|l| l.trim_matches([' ', '|']))
                .collect::<String>();
            assert!(joined.contains(&"9".repeat(78)), "{shown}");
        }
        let work = app.update(Msg::Key(Key::Char('a')));
        let [Effect::Tasks(id)] = work.as_slice() else {
            panic!()
        };
        assert!(app.tasks.report.is_none());
        assert!(matches!(
            app.take_task_job(*id).unwrap().request,
            Request::Report { reveal: true, .. }
        ));
    }
    #[test]
    fn tools_menu_tasks_are_reachable_on_both_families_and_do_not_shadow_tools() {
        for chain in ["ethereum", "tron"] {
            let mut app = app(chain);
            app.nav.tab = Tab::Tools;
            let work = app.update(Msg::Key(Key::Char('a')));
            assert!(matches!(work.as_slice(), [Effect::Tasks(_)]));
            app.update(Msg::Key(Key::Esc));
            assert!(!app.tasks_visible());
            app.start_tool(super::super::tools::Tool::Call);
            assert!(!app.tasks_visible());
        }
    }
    #[test]
    fn submitted_task_create_cannot_be_cancelled_or_lose_its_result() {
        let mut app = app("ethereum");
        app.start_tasks(ActionId::TaskCreate);
        type_text(&mut app, "save-1");
        app.update(Msg::Key(Key::Enter));
        app.update(Msg::Key(Key::Enter)); // No optional allocation.
        let work = app.update(Msg::Key(Key::Enter));
        let [Effect::Tasks(id)] = work.as_slice() else {
            panic!()
        };
        assert!(app.mutation_pending());
        app.update(Msg::Key(Key::Esc));
        assert!(app.tasks.creating);
        app.update(Msg::Key(Key::Char('q')));
        assert!(app.quit_pending() && !app.quitting());
        assert!(
            app.update(Msg::Tasks(*id, TaskMsg::Created(Ok(summary()))))
                .is_empty()
        );
        assert!(app.quitting());
    }
    #[test]
    fn transfer_attribution_rebuilds_owned_review_requests_on_both_families() {
        use super::super::transfer::{Stage as SendStage, TransferJob, tron::TronJob};
        use crate::core::whitelist::{Entry, ListFiles, Whitelist};
        use crate::wallet::{
            TransferInput, TransferOptions,
            tron_send::{TronAsset, TronTransferInput},
        };
        for chain in ["ethereum", "tron"] {
            let mut app = app(chain);
            let evm_from = "0x1111111111111111111111111111111111111111";
            let from = if chain == "tron" {
                chains::tron::address::from_body(&[0x11; 20])
            } else {
                evm_from.into()
            };
            app.verified_indices.insert("selected-key".into(), 2);
            app.nav.family_pins.insert(
                ("selected-key".into(), app.chain.family.name()),
                from.clone(),
            );
            app.keys = Pane::Ready(KeyList {
                rows: vec![KeyLine {
                    key: "selected-key".into(),
                    name: Outside::key_name("selected-key"),
                    index: 2,
                    address: Some(Outside::address(evm_from)),
                    active: false,
                }],
                warning: None,
            });
            app.status = Pane::Ready(Bar {
                configured: true,
                key: None,
                address: None,
                trust: None,
                lock: LockLine::Open(None),
            });
            let mut list = Whitelist::empty(&chains::WHITELIST_RULES);
            list.add(Entry::new_address(
                "alice",
                "0x2222222222222222222222222222222222222222",
                &[chain],
            ))
            .unwrap();
            let checked = CheckedList {
                list,
                source: "fixture",
                fingerprint: ListFiles::read(Path::new("/tmp/zwt-task-ui-no-list")).fingerprint(),
            };
            app.checked_list = Some(checked.clone());
            app.nav.tab = Tab::Send;
            app.transfer.stage = SendStage::Review;
            if chain == "tron" {
                app.transfer.tron.draft = Some(TronTransferInput {
                    task_id: None,
                    key_name: "selected-key".into(),
                    index: 2,
                    from,
                    payee: crate::wallet::tron_send::tron_transfer_payees(&checked).remove(0),
                    asset: TronAsset::Trx,
                    amount: "1".into(),
                    allow_duplicate: false,
                });
            } else {
                app.transfer.draft = Some(TransferInput {
                    key_name: "selected-key".into(),
                    index: 2,
                    from,
                    payee: crate::wallet::transfer_payees(&checked, app.chain).remove(0),
                    amount: "1".into(),
                    token: None,
                    options: TransferOptions::default(),
                });
            }
            app.update(Msg::Key(Key::Char('t')));
            assert_eq!(app.transfer.stage, SendStage::Task);
            type_text(&mut app, "task-1");
            let work = app.update(Msg::Key(Key::Enter));
            let [Effect::Transfer(id)] = work.as_slice() else {
                panic!("{work:?}")
            };
            let job = app.take_transfer_job(*id).unwrap();
            // Editing the mutable draft after dispatch cannot mutate the owned worker request.
            if let Some(draft) = app.transfer.draft.as_mut() {
                draft.options.task_id = Some("different".into());
            }
            if let Some(draft) = app.transfer.tron.draft.as_mut() {
                draft.task_id = Some("different".into());
            }
            match job {
                TransferJob::Review { input, .. } => {
                    assert_eq!(input.options.task_id.as_deref(), Some("task-1"));
                    assert_eq!(input.key_name, "selected-key");
                    assert_eq!(input.index, 2);
                }
                TransferJob::Tron(TronJob::Review { input, .. }) => {
                    assert_eq!(input.task_id.as_deref(), Some("task-1"));
                    assert_eq!(input.key_name, "selected-key");
                    assert_eq!(input.index, 2);
                }
                _ => panic!("attribution must prepare, never send"),
            }
        }
    }
}
