# Screen Receiver (CCPC-Backend)

A lightweight Rust backend that receives screen frames from remote senders over a custom TCP protocol and serves a web dashboard + live stream over WebSockets.

This repository implements the receiver/server side. It accepts incoming TCP connections from devices that stream encoded frames, keeps per-device metadata and the latest frame in memory, and broadcasts updates to connected browser clients via a WebSocket API. A static web UI (in `static/`) provides a dashboard and a pop-up live viewer.

Key features
- TCP server for device/frame ingestion (default port 6082)
- Web server and WebSocket endpoint for dashboard & live streaming (default port 4325)
- Device registry with online/offline detection
- Broadcast channel to push frame headers and binary frames to WebSocket clients
- Simple, modern dashboard and a dedicated stream viewer in `static/`

Quick links
- Source: `src/main.rs`
- Static UI: `static/dashboard.html`, `static/stream.html`
- Manifest: `Cargo.toml`

Prerequisites
- Rust toolchain (rustc + cargo). Install from https://rustup.rs
- On Windows, the project uses `winres` to embed an icon when building; keep that in mind if you modify packaging.

Build

1. Open a terminal in the repository root (this project uses the Rust toolchain):

	```powershell
	cargo build --release
	```

Run

1. Run the server (debug):

	```powershell
	cargo run
	```

2. Or run the release binary:

	```powershell
	cargo run --release
	```

3. Open the dashboard in a browser at:

	- http://localhost:4325/ (the server prints the web port when it starts)

How it works (high-level)

- TCP ingestion:
  - Devices connect to the TCP server.
  - Protocol begins with 4 bytes ASCII `STRM` then a 4-byte version, then a 4-byte device-info size followed by a JSON device info blob.
  - Each frame is sent as: 4-byte header-size, header JSON (FrameHeader), then binary frame payload of length `compressed_size`.
- Web UI:
  - The server hosts a web UI and a WebSocket endpoint used by the dashboard and stream viewer.
  - Frame delivery to browsers is done in two parts: a `frame` JSON header (metadata) followed by a binary message containing the JPEG/encoded payload. The frontend links the two to render frames.

Default configuration
- TCP ingestion port: 6082
- Web server port: 4325
- Offline timeout and cleanup are set in code (see `ServerConfig::default()` in `src/main.rs`).

Project layout

- Cargo.toml — Rust manifest and dependency list.
- src/main.rs — main server implementation (TCP server, device management, and Warp-based web server + WebSocket handlers).
- static/
  - dashboard.html — the dashboard UI (device list, stats, and stream modal)
  - stream.html — dedicated live stream viewer (pop-out)

Notes for developers
- Concurrency: the server uses threads for TCP client handling and Tokio for the web server. Shared state is wrapped in Arc<Mutex<...>>.
- Broadcasts: `tokio::sync::broadcast` is used to push messages (headers and binary frame notifications) to WebSocket handlers.
- Safety: the code performs basic sanity checks (header sizes, timeouts), but you should validate input and add size limits before production use.

Extending and debugging tips
- To change ports or timeout values, modify `ServerConfig::default()` in `src/main.rs`.
- To inspect incoming frames while running, add logging in `handle_client_connection` where frames are read and processed.
- If you want persistent storage of device metadata or frames, replace the in-memory maps with a database-backed layer.

Troubleshooting
- "Port already in use": another process is listening on the configured TCP or web port. Stop that process or change ports.
- "Invalid protocol header": ensure the sender begins the stream with `STRM` (4 ASCII bytes) and follows the header conventions used in this project.

Contributing
- Contributions are welcome. Please open issues or PRs describing bug fixes or feature additions. If adding features that affect the wire protocol, document the changes and keep backward compatibility in mind.

License
- This repository does not include a license file. Add one (for example, MIT or Apache-2.0) to make usage terms explicit.

Contact
- If this is part of a team or project (CCPC), add contact or maintainer information here.

----
Small reproduction / dev quick-start

1. Build and run: `cargo run`.
2. Open the dashboard: `http://localhost:4325/`.
3. Use a sender that follows the `STRM` protocol to stream frames to port 6082, or adapt a simple test sender.

If you'd like, I can also:
- Add a small Rust or Python test sender script that follows the protocol to stream a sample JPEG to the server for local testing.
- Add a CI step or a basic `cargo check` step in the README.

Enjoy!