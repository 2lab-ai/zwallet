# GitHub issue #55 source snapshot

Source: https://github.com/2lab-ai/zwallet/issues/55
Fetched: 2026-10-30
State at fetch: open
Title: CI flakes at d6fb99d: whitelist test matches '2J' in a random temp path; tr4 lost its session to the in-place edit_wallet (#52)

The source body below is preserved verbatim. It records the handoff author's
observations and requests at that time; current execution status and authorization
are recorded in the driver and completion checklist.

---

Main CI at d6fb99d ([run 40971807359](https://github.com/2lab-ai/zwallet/actions/runs/40971807359)) failed on both runners, each in a different test. The same source had passed at 599bde9; d6fb99d only changed `.prd/` docs.

## macOS: `core::whitelist::tests::a_list_file_with_a_value_outside_zwallets_shapes_is_no_list_and_is_never_echoed`

The panic is at `src/core/whitelist.rs:3246`:

```
assert!(!msg.contains("##") && !msg.contains("2J"), "{what}: {msg}");
```

`msg` contains the list file's path, and that path sits under a random temp directory. In this run the directory was named `.tmpax2JkL`, which contains `2J`. The assertion is meant to show that the planted value `## ##\u{1b}[2J` is never echoed. Instead it matched two letters of the random directory name. The message itself was correct: `the nickname of entry 1 is not in the shape zwallet writes`.

- **How often:** a `.tmp` + 6 alphanumeric name contains a given 2-character pair in about 5 of 3,844 runs, roughly 0.13% per run.
- **Fix:** assert on what the value would actually leave if echoed, namely the escape character and `[2J`, not on the bare `2J`. Sweep the repo for any other negative substring assertion that a random path can satisfy.
- **Deterministic RED:** set `TMPDIR` to a directory whose name contains `2J`.

## Ubuntu: `tr4_a_tampered_tron_cache_is_refused_and_shown_escaped`

```
expected failure; stdout {"address": "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C", "chain": "tron", "key": "main", "verified": false}
```

The printed address is `TRON_POISON`. tr4 plants it with `home.edit_wallet` right after `home.unlock("10m")` and expects the live session to contradict it. `verified: false` means no session answered. So the session was gone when the CLI asked, and the CLI fell back to the cache, as designed when there is no session.

That is the #52 mechanism. The old `edit_wallet` rewrote wallet.json in place, a daemon reading at that moment saw a torn file, and it ended the session. #54 (merged as 6f1a2c9) made `edit_wallet` an atomic rename, which closes this. No product change is needed.


