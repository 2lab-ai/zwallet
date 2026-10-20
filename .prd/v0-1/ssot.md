# v0.1 — SSOT

Status: **in-progress**, 2026-10-20. This is the fixed target; do not reinterpret it during the loop.

## User statement (verbatim, 2026-10-20, in order)

1. "https://github.com/trustwallet/tw-agent-skills 이걸로 지갑 만들고 이더리움 주소 줘 네가 필요할때 사용할수 있는 소액 넣어줄꺼야"
2. "뭔 씨발 좇같이 로컬에서 지갑만드려는데 로그인을 요구함? 병신임?"
3. "아니 맥키체인에 저장하지말고 씨발 그냥 너만 사용할수 있으면 된다고"
4. "트러스트 월릿 씨발새끼네ㅐ 그럼"
5. "쓰지마 병신이네"
6. "트러스트월릿 포크해서 좇같은거 걷어내고 러스트로 포팅해 시발 그게 뭐 얼마나 대단하거라고"
7. "트러스트월릿 관련 ㅈ가업한거 다 걷어내 개 병신이네"
8. (WDK CLI report pasted, then) "음 왜 데몬을 따로 만들었지? zwallet으로 진짜 기본기만 넣어서 쓰는게 나을것 같기도 하네"
9. "그냥 지갑 생성 해당 지갑 밸런스 조회하고 함수 호출하고 전송만 되면 되는데 데몬이 왜 필요함?"
10. "지원: 이더리움만 대신에 솔라나 톤은 나중에 추가할꺼니까 아키텍쳐에 확장 고려해서 만듬"
11. "아 잠깐만 그게 나을것 같기도하네 한번 사람이 시간을 두고 풀어주고 데몬이 그 시간동안 키를 가지고 있는 설계는 좋은거 같아 원하면 그 언락을 24시간 혹은 365일을 할수도 있는거니까"
12. "키만 그렇게 처리해줘"
13. "그러니까 사람이 zwallet lock을 하면 잠기는거고 zwallet unlock 24h, zwallet unlock 365d나 zwallet unlock 1y 할수 있도록"
14. "그리고 이 zwallet은 llmux 개발 ㄱ거 확인해서 brew에도 올리고 xbrew에도 올려줘"
15. "그리고 /using-dotprd 사용해"
16. "1. 그 마스터키를 만들고 마스터키에서 서브키 만들수 있는거 그걸로해줘 2. 그리고 키를 그럼 여러개 서로 변경할 수 있도록해줘 (새로 키를 만들면 해당 키로 변경됨) 3. zwallet list, zwallet change {키이름} 추가"
17. "니모닉은 디폴트로 좇나 긴걸로해주고"
18. "ai내성이나 양자 내성 월릿을 사용할수 있으면 그걸로해줘 처음부터"

## Fixed scope

- **Functions** (9): create, balance, function call (read `call`, write `invoke`), transfer. Nothing
  else.
- **Key handling** (3, 11, 12, 13): no OS keychain. A human unlocks for a positional duration
  (`zwallet unlock 24h|365d|1y`) and locks with `zwallet lock`. A daemon holds only the key during
  that time; everything else is a plain one-shot CLI.
- **Keys** (16, 17): one HD master (BIP39, **24 words by default**) → named subkeys by index.
  `zwallet create <name>` creates the master the first time, and after that derives the next
  subkey; either way the new key becomes active. `zwallet list` shows the keys and
  `zwallet change <name>` switches the active one. The CLI is flat (`zwallet <verb>`).
- **Quantum / AI resistance** (18): use it from the start if it is usable under the no-login,
  public-RPC constraint. Under research 2026-10-20, and the decision is recorded in 02-architecture
  §Resistance.
- **Chains** (10): EVM only in v0.1. The architecture leaves room for Solana and TON.
- **No Trust Wallet anything** (5, 6, 7): no Trust API, packages, keychain, or telemetry. twak's
  source is not public, so "fork" = re-implement its wallet surface (see 01-spec §Problem).
- **Distribution** (14): the llmux pattern — CI, preview, release, `2lab-ai/homebrew-tap` formulae
  (stable and preview), and an xbrew recipe.
- **Deliverable to the user** (1): an Ethereum address the agent can use, from a wallet the user
  created and unlocked on this machine.
