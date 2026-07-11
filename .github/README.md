# GitHub Configuration for Void

This directory contains GitHub-specific configurations for the Void project:

## 📁 Contents

### Workflows (`workflows/`)

Automated GitHub Actions workflows for CI/CD and checks:

- **`cla-check.yml`** — Verifies that PR contributors have signed the CLA before merge (runs on `pull_request_target` so it also works for PRs opened from forks)
- Other CI/CD workflows for testing, building, and linting (as configured)

### Issue & PR Templates

- **`ISSUE_TEMPLATE/`** — Structured issue forms:
    - `bug_report.yml` — Bug reports (repro steps, environment, logs)
    - `feature_request.yml` — Feature requests and enhancements
    - `config.yml` — Template chooser config (enables blank issues, links to CONTRIBUTING.md)
- **`PULL_REQUEST_TEMPLATE.md`** — Template for new pull requests

### Documentation

- **`CLA_SETUP.md`** — Complete setup guide for CLA Assistant configuration and troubleshooting

## 🚀 Quick Start

### For Contributors

1. Read [CONTRIBUTING.md](../CONTRIBUTING.md)
2. Read the [CLA](../CLA.md)
3. Open an issue or PR using the provided templates

### For Maintainers

1. Configure CLA Assistant following [CLA_SETUP.md](./CLA_SETUP.md)
2. Enable branch protection rules requiring CLA verification
3. Enable **Private vulnerability reporting** (Settings → Security) so the security reporting link in CONTRIBUTING.md works
4. Monitor PR checks and contributor signatures

## 📋 Workflows Explained

### CLA Check

**Trigger:** Every PR opened or updated (via `pull_request_target`, so forked PRs are covered too)

**Actions:**
1. Extract PR author
2. Skip check for bots (dependabot, renovate)
3. Verify CLA signature via CLA Assistant
4. Add comment requesting signature if needed
5. Report status to GitHub

**Status:** Shows as a required check before merge

## 🔗 Related Files

- [`../CONTRIBUTING.md`](../CONTRIBUTING.md) — Contributor guidelines
- [`../CLA.md`](../CLA.md) — Contributor License Agreement
- [`../signatures/cla.json`](../signatures/cla.json) — Signed CLA records

---

For questions or improvements, open an issue or refer to the main [README](../README.md).