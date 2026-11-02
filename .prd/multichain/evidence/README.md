# Multichain observation evidence

The six receive PNGs render the **tested ratatui framebuffer**, not a physical camera
capture. The matching JSON records the actual buffer, QR rectangle and address start
coordinates, including all trailing cell padding. Text companions trim trailing
spaces for readable diffs; JSON remains the full framebuffer. `tests/tui_multichain.rs` decodes the rendered modules with rqrr, asserts
four-module quiet zones, verifies every address character and rejects unverified or
impostor identity. The TON address is the independent SDK mixed-hex fixture (41×21
QR at 80×24), not a numerically compressible placeholder.

Tron: QR (0, 1), 37×19; full 34-character address at (39, 4), in one horizontal row at both
sizes. TON: QR (0, 1), 41×21; full 66-character address starts (43, 4), wraps at 80 columns
and fits one row at 120 columns. JSON files carry exact data for each case.

`zwallet-tron-qr-pty-*.txt` are actual PTY observations at both sizes, with the address
on the QR's same row and terminal restoration. `zwallet-native-tui-*.txt` are actual
Solana/TON PTY transfer observations: the opened key signs while another global key
stays active; wrong nickname sends nothing, separate exact confirmation submits to
the loopback mock. Fixtures are public test identities; no owner wallet/funds were used.

The final source/test manifest and local results are recorded in `validation.json`.
PR #59 checks and issue #60's final receipt hold post-commit Linux/macOS CI results,
so no source document claims a future self-referential commit was tested.
