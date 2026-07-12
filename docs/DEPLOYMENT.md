# Deployment

Void runs on a single-node **k3s** cluster on an Oracle Cloud Ampere A1 (ARM64) VM. There are two environments on that same node, kept isolated by namespace and by port:

| | Production | Staging |
|---|---|---|
| **Namespace** | `void` | `void-staging` |
| **Hostname** | `api.${DOMAIN}` (e.g. `api.voidsfu.com`) | `staging-api.voidsfu.com` |
| **HTTP port** | 3001 | 3002 |
| **ICE/UDP port** | 10000 | 10001 |
| **Manifest** | [`deployment-k3s.yaml`](../deployment-k3s.yaml) | [`deployment-k3s-staging.yaml`](../deployment-k3s-staging.yaml) |
| **Monitoring stack** | Prometheus, Grafana, Alertmanager | none — signaling-server only |
| **Trigger** | push a `v*` tag | push to `main` |

## Why two ports

The signaling server binds with `hostNetwork: true` so its UDP ICE mux is reachable directly from external WebRTC peers (no Docker/k8s bridge NAT in the way). Because both environments run as separate pods **on the same physical node**, they can't share a host port — hence staging is shifted to 3002 (HTTP) / 10001 (UDP) via the `PORT` and `ICE_UDP_PORT` environment variables the Rust binary already reads at startup. Kubernetes' `containerPort`/`hostPort` fields are just metadata for kube-proxy/documentation purposes — they don't change what the binary actually binds. Whatever port the app is told to bind (via env) has to match the manifest's `hostPort`, or the container will fail to start with `AddrInUse`.

## CI/CD flow

[`deploy-signaling.yml`](../.github/workflows/deploy-signaling.yml) builds the same Rust binary and the same Docker image (pushed to GHCR) regardless of target — only the deploy step branches:

- **Push to `main`** → renders `deployment-k3s-staging.yaml`, applies it to the `void-staging` namespace, health-checks `localhost:3002`.
- **Push a tag matching `v*`** → renders `deployment-k3s.yaml`, applies it to the `void` namespace, health-checks `localhost:3001`, also rolls out the monitoring stack.

Both paths reuse the same GHCR image — nothing is rebuilt twice. The job determines which branch to take from `github.ref` at runtime (see the "Determine deploy target" step in the workflow).

## TLS certificates

Both namespaces share the same cluster-wide `letsencrypt-prod` `ClusterIssuer` (cert-manager, Let's Encrypt via HTTP-01 through Traefik) — `ClusterIssuer`s aren't namespace-scoped, so there's no need for a second one. Each namespace has its own `Certificate` resource and its own secret (`void-tls-certs` / `void-staging-tls-certs`).

## Secrets

Kubernetes secrets don't cross namespaces, so anything the pod needs has to be upserted in **both** `void` and `void-staging` separately. The GitHub Actions secrets that feed them:

| Secret | Used for |
|---|---|
| `PRIMARY_PIN_HASH`, `BACKUP_PIN_HASH` | Compiled into the binary at build time (TLS cert pinning, see [SECURITY.md](./SECURITY.md)) |
| `ORACLE_HOST`, `ORACLE_SSH_KEY` | SSH access to the VM for the deploy step |
| `DOMAIN` | Prod hostname template (`api.${DOMAIN}`) |
| `PUBLIC_IP` | Oracle VM's public IP — same for both environments, used for 1:1 NAT in WebRTC |
| `JWT_SECRET` | Prod session signing key |
| `STAGING_JWT_SECRET` | Staging session signing key — **deliberately different** from prod, so a token issued by staging is never valid against prod and vice versa |
| `GHCR_TOKEN` | Long-lived PAT so k3s can pull the private image (both namespaces need their own `ghcr-secret`, refreshed on every deploy) |
| `GF_ADMIN_USER`, `GF_ADMIN_PASSWORD` | Grafana admin credentials (prod only) |
| `SLACK_WEBHOOK_URL` | Deploy notifications |

## DNS & firewall (Oracle Cloud security list)

DNS is managed at the registrar (A records pointing at the VM's public IP). Ports currently open on the VM's security list:

- `22/tcp` — SSH
- `80/tcp`, `443/tcp` — HTTP/HTTPS (Traefik, ACME HTTP-01 challenges)
- `3001/tcp`, `3002/tcp` — signaling server direct access (prod / staging)
- `3000/tcp`, `9090/tcp`, `9093/tcp` — Grafana / Prometheus / Alertmanager (prod only)
- `10000-20000/udp` — WebRTC ICE media (covers both prod's 10000 and staging's 10001)

## Testing across devices without a second dev environment

See the ["Testing Real-Time Features on a Second Device"](../CONTRIBUTING.md#testing-real-time-features-on-a-second-device) section of `CONTRIBUTING.md` — contributors can point a local build at the staging endpoint, or run the [`Staging Test Build`](../.github/workflows/staging-test-build.yml) workflow from their own fork to get cross-platform (Windows/macOS/Linux) test binaries without installing the full Rust/Tauri toolchain on a second machine.
