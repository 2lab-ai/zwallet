//! Worker bridge for creation. Plans and owned password buffers stay outside UI messages.
use super::*;
use app::create::{CreateEffect, CreateMsg};

pub(super) fn start(
    effect: CreateEffect,
    home: &Path,
    tx: &UnboundedSender<Msg>,
    allow_ptrace_scope_0: bool,
    pending: &Arc<Mutex<Option<wallet::CreatePlan>>>,
    secret: &Arc<Mutex<Option<PasswordBuf>>>,
) {
    let home = home.to_path_buf();
    let tx = tx.clone();
    match effect {
        CreateEffect::Prepare {
            id,
            name,
            words,
            ttl,
        } => {
            let pending = pending.clone();
            tokio::task::spawn_blocking(move || {
                let result = wallet::prepare_create(&home, &name, words, ttl, allow_ptrace_scope_0)
                    .map(|plan| {
                        let first = plan.creates_master();
                        *pending.lock().expect("creation plan lock") = Some(plan);
                        first
                    });
                let _ = tx.send(Msg::Create(CreateMsg::Prepared(id, result)));
            });
        }
        CreateEffect::Submit { id } => {
            let plan = pending.lock().expect("creation plan lock").take();
            let password = secret.lock().expect("secret handoff lock").take();
            tokio::spawn(async move {
                let result = match (plan, password) {
                    (Some(plan), Some(password)) => wallet::create(plan, password).await,
                    _ => Err(WalletError::validation(
                        "creation plan or password is missing",
                    )),
                };
                let _ = tx.send(Msg::Create(CreateMsg::Finished(id, result)));
            });
        }
    }
}
