# Review hardening — fixed scope

Date: 2026-10-29

## User instructions (verbatim)

> 니가 말한 내용들 깃허브에 이슈로 만들고 니가 픽스해줘.
>
> using-dotprd 스킬 사용해

> 아마도 직접 pr을 올리고 머지는 못하고 fork해서 ai-insightquest 계정으로 pr 올려

> 기존 원칙 유지: 한계를 명시하고 설계 이슈로 추적

## Scope

Track every actionable item from the review of 9ecfe932b0feb6427290125091e959e281135328 as GitHub issues. Implement receipt deadline enforcement, CI-gated publishing, the Clippy fix, bounded daemon filesystem checks, and optional stateless spending/calldata policy limits. Preserve custody invariants. Cumulative spending and joint rollback remain explicit design issues, as the user chose not to introduce a trusted external state boundary. Open a PR from ai-insightquest's fork; do not merge or deploy.

Source review evidence: 492 existing tests passed, 11 live tests ignored; Rust 1.95 Clippy failed at src/tui/app.rs:208; a 100ms receipt deadline waited 1.51s against a local mock RPC. Filesystem stall concern is static analysis, requiring a regression seam.

## Additional user instruction (verbatim)

> 그리고 항상 커밋에는 co-author로 z@2lab.ai 추가해줘
> 이거 개인 agents.md에 추가해줘 이거

All commits include `Co-authored-by: Z <z@2lab.ai>`. Personal AGENTS.md updated on 2026-10-29.

## Documentation update instruction (verbatim, 2026-10-29)

> 문서 업데이트했거든 리베이스하고 문서까지 포함해서 pr 업데이트

Rebase PR #29 onto upstream main `51b4a51b4fae511c726f7f1be0d009c0360fe264`.
Preserve the README / docs split and WIP labels from PR #30; reconcile command,
security, feature-status and development documentation with this PR's implementation.
The existing fork, no-merge/no-deploy and co-author instructions still apply.

## CI completion correction (verbatim, 2026-10-29)

> 야 니가 알아서 해야지 실패 났잖아 ㄴ시발 이걸 내가 말해줘야함?

Complete PR delivery by inspecting failed checks, fixing their cause, and verifying
both Linux and macOS CI on the updated PR head before reporting completion.
Local success or a queued run is not the delivery gate. Existing fork, attribution,
and no-merge/no-deploy instructions remain in force.
