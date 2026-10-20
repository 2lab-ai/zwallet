# zwallet

Local, login-free EVM wallet CLI for agents. A Rust re-implementation of the wallet surface of
Trust Wallet's `twak` CLI with everything that needed Trust's servers removed: no API credentials,
no telemetry, no OS keychain, no backend registration. Keys stay on this machine; chain access is
plain JSON-RPC.
