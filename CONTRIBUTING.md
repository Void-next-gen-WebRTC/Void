# Contributing to Void

Thank you for considering contributing to Void! We appreciate your interest in helping us build a better, open, and distributed communication system.

---

## Code of Conduct

We are committed to providing a welcoming and inspiring community for all. Please read our implicit expectations:
* **Be respectful** — Disagreement is natural, but remain professional.
* **Be inclusive** — Welcome all backgrounds and perspectives.
* **Be patient** — Help others learn and grow.

---

## Getting Started

### Prerequisites
* **Node.js LTS** and **pnpm v9+** (do not use npm or yarn)
* **Rust (stable toolchain)** (for `core-wasm`, `desktop` and `signaling-server`)
* **Git 2.30+**

### Local Setup

```bash
# Clone the repository
git clone https://github.com/Void-next-gen-WebRTC/Void.git
cd Void

# Install dependencies
pnpm install

# Build the WASM core bindings
cd packages/core-wasm
wasm-pack build --target web --out-dir ../../apps/desktop/src/pkg

# Return to root and start the desktop app in development mode
cd ../../apps/desktop
pnpm dev
```

### Project Structure

Before contributing, please familiarize yourself with our monorepo architecture (see [README.md](./README.md)):

* **apps/desktop** — Tauri v2 + React 19 + Vite 7 desktop application
* **packages/core-wasm** — Rust → WASM DSP (SmartGate, RNNoise), codec, video, and network scoring
* **packages/signaling-server** — Rust SFU (webrtc-rs) + WebRTC signaling, auth, and friends modules

---

## Contribution Workflow

### 1. Fork & Branch

```bash
git checkout -b feature/your-feature-name
```

Use descriptive branch names with one of the following prefixes:

* `feature/` — New functionality
* `fix/` — Bug fixes
* `docs/` — Documentation
* `refactor/` — Code refactoring
* `test/` — Test additions

### 2. Code Standards

#### TypeScript / React (Frontend)

* **Styling & Icons:** Use TailwindCSS v4 for styling and lucide-react for icons.
* **File Length:** Maximum 350 lines per file — extract complex state or business logic into dedicated hooks or contexts.
* **Types:** No loose types or inline interfaces — place shared schemas in `.models.ts` or `.types.ts` files.
* **Documentation:** Use JSDoc comments for public helper functions and custom context providers.
* **Naming:** Prefer `camelCase` for UI variables and `kebab-case` for file/directory names.
* **Comments:** Comment code only when architectural logic is non-obvious; avoid over-commenting.

#### Rust (Backend, WASM, Signaling)

* **Formatting:** Code must be formatted using `cargo fmt`.
* **Linting:** Code must compile without warnings under `cargo clippy`. Avoid raw `.unwrap()` or `.expect()` calls in media processing pipelines.
* **Naming:** Use standard `snake_case` for variables/modules and `PascalCase` for types/traits.
* **Documentation:** Document all public modules, constants, and endpoints with `///` comments (to support `cargo doc`).
* **Testing:** Add appropriate unit tests inside your module or integration tests in a nested `tests/` workspace.

#### General Rules

* All code comments, commit messages, and issues must be in English.
* Keep commits atomic (one logical change per commit) and highly descriptive.
* No hardcoded secrets, testing API keys, or deployment credentials.

### 3. Commit Message Format

We strictly follow the **Conventional Commits** specification. Here is an example of a perfect commit:

```
feat: Add voice activity detection threshold control

- Add configurable VAD threshold in audio context
- Update RNNoise parameters to respect threshold
- Add UI slider for user control

Refs #42
```

Allowed prefixes:

* `feat:` — New feature
* `fix:` — Bug fix
* `docs:` — Documentation changes
* `style:` — Formatting, missing semi-colons (no logic change)
* `refactor:` — Code reorganization or optimization
* `test:` — Test additions or modifications
* `perf:` — Performance improvements
* `chore:` — Build process, CI configuration, or dependency updates

### 4. Local Testing

Before submitting a Pull Request, ensure that the full verification pipeline passes locally to prevent CI runner failures:

```bash
# Run the complete Rust workspace test suite (all 4 crates)
cargo test --workspace

# Run TypeScript type checking
pnpm --filter desktop exec tsc --noEmit

# Run the frontend Vitest suite
pnpm --filter desktop test:run

# Static analysis and formatting checks
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

---

## Pull Request Process

### Before Opening a PR

1. Ensure your branch is fully rebased and up-to-date with `main`.
2. Run all tests locally.
3. **Sign the CLA:** Open your PR, and our automated CLA Verification workflow will evaluate your signature. Follow the instructions posted by the bot directly inside your PR discussion.

### Opening the PR

1. Use the provided Pull Request template (automatically loaded).
2. Link related issues in the description using standard GitHub keywords: `Closes #123` or `Refs #456`.
3. Provide a clear description of your changes, your technical design decisions, and why they matter.
4. Add screenshots or structural GIFs if your change introduces visual modifications to the React UI layout.
5. Request reviewers (maintainers will be automatically notified).

### PR Review

* Expect initial engineering reviews within 3–7 days.
* Be open to architectural feedback; code review is a collaborative process designed for collective learning.
* If changes are requested, simply push new commits to your existing branch.
* Once fully approved and all automated checks turn green, a maintainer will merge your PR.

### After Merge

* Your contribution is now a permanent part of Void! 🎉
* You will be credited via global Git history.

---

## Reporting Issues

### Before Reporting

* Search the repository's open and closed issues to avoid creating duplicates.
* Update your local Void workspace to the latest commit on `main` to verify if the bug still persists.

### Reporting Security Issues

⚠️ **CRITICAL:** Do NOT open a public GitHub issue for security vulnerabilities, memory exploits, or protocol leaks. Please report them confidentially using **[GitHub Private Vulnerability Reporting](../../security/advisories/new)** (repository's **Security** tab → **Report a vulnerability**). This keeps the report private between you and the maintainers until a fix is released.

### Creating a Standard Issue

1. Use the auto-filled Issue template.
2. Provide a comprehensive description of the problem.
3. Include minimal, step-by-step instructions to reliably reproduce the bug.
4. Describe both the expected and actual behavior.
5. Include your system architecture details (OS, CPU, Void version, relevant hardware).
6. Attach console trace logs or terminal crash logs if available.

---

## Legal & Licensing

By contributing to Void, you agree to the terms of the **[Contributor License Agreement (CLA)](./CLA.md)**. The CLA ensures that:

* You have the legal right to contribute your code.
* Your contribution can be freely used under the project's license model.
* You retain moral authorship credit for your work.

The project is distributed under the **Business Source License 1.1 (BSL-1.1)**, which will automatically transition to **GPL-3.0-or-later** on April 7, 2031. Your contributions are governed under this exact same license.

---

## Questions?

* **Documentation:** See [README.md](./README.md) and inline specifications.
* **Issues:** Use GitHub Issues for bug reports and official feature requests.

---

Thank you for contributing to Void! Your effort helps us build the open, reliable communication system we believe in. 💜