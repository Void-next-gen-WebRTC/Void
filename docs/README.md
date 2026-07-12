# Void — Documentation

This is the documentation hub. Start here, then dive into whichever part you need — most of the detailed technical writing lives next to the code it describes, not duplicated here.

## Where things live

| Topic | Where |
|---|---|
| **Project overview, tech stack, key flows** | [root README](../README.md) |
| **API reference (generated from source)** | [docs.rs-style rustdoc, published from CI](https://void-next-gen-webrtc.github.io/Void/) |
| **Desktop app (React + Tauri)** | [apps/desktop/README.md](../apps/desktop/README.md) |
| **WASM DSP core** (RNNoise, SmartGate, codec, network scoring) | [packages/core-wasm/README.md](../packages/core-wasm/README.md) |
| **SFU engine** (WebRTC, ICE/SDP, RTP/SCTP forwarding) | [packages/void-sfu/README.md](../packages/void-sfu/README.md) |
| **Signaling server** (auth, friends, presence, WS signaling) | [packages/signaling-server/README.md](../packages/signaling-server/README.md) |
| **Deployment & infrastructure** (k3s, staging vs prod, CI/CD) | [DEPLOYMENT.md](./DEPLOYMENT.md) |
| **Security model** (TLS pinning, vulnerability reporting, CLA) | [SECURITY.md](./SECURITY.md) |
| **Contributing** (workflow, code standards, testing, CLA) | [CONTRIBUTING.md](../CONTRIBUTING.md) |

## Generated API docs

The `rustdoc` reference for `core-wasm`, `void-sfu`, `signaling-server`, and the Tauri desktop backend is rebuilt and published automatically on every push to `main` — see [`.github/workflows/docs.yml`](../.github/workflows/docs.yml). It's the equivalent of `cargo doc --open` for the whole workspace, without needing to build it locally.
