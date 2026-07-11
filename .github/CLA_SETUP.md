# CLA Setup Guide for Void

This guide explains how to configure the Contributor License Agreement (CLA) verification in your GitHub repository.

## What is CLA Assistant?

**CLA Assistant** is a GitHub App that automatically checks if contributors have signed your Contributor License Agreement before their PRs can be merged. It:

1. Comments on new PRs requesting CLA signature
2. Tracks signed agreements in `signatures/cla.json`
3. Reports the status as a GitHub Status Check
4. Blocks merging until the CLA is signed

## Setup Instructions

### Step 1: Install CLA Assistant

1. Go to [CLA Assistant on GitHub Marketplace](https://github.com/apps/cla-assistant)
2. Click **"Install"**
3. Select your organization/account and the `Void` repository
4. Grant necessary permissions

### Step 2: Configure Repository Rules

Once CLA Assistant is installed, configure it as a **required status check**:

1. Go to **Settings** → **Branches** → **Branch protection rules**
2. Click **Add rule** or edit the rule for `main`
3. Under **Require status checks to pass before merging**, search for and enable:
   - `cla-assistant` (or `cla-assistant-lite` depending on your installation)
   - Any other required CI checks (tests, linting, etc.)
4. Save the rule

### Step 3: GitHub Actions Workflow

This repository already includes a GitHub Actions workflow (`.github/workflows/cla-check.yml`) that:

- Triggers on every new PR and update
- Calls CLA Assistant to verify the contributor's CLA signature
- Automatically skips bots (dependabot, renovate)
- Reports the status back to GitHub

**No additional configuration needed** — the workflow runs automatically.

### Step 4: Customize Messages (Optional)

The CLA Assistant workflow supports custom messages. Edit `.github/workflows/cla-check.yml` to customize:

- **`custom-notsigned-prcomment`** — Message when CLA is not signed
- **`custom-pr-sign-comment`** — Message when contributor signs
- **`custom-allsigned-prcomment`** — Message when all contributors have signed

## How It Works for Contributors

1. **Contributor opens a PR** → CLA Assistant comment appears asking to sign the CLA
2. **Contributor reads the CLA** → They reply to the comment: *"I have read the CLA and agree to its terms"*
3. **CLA Assistant verifies** → Their signature is recorded in `signatures/cla.json`
4. **Status Check passes** → GitHub displays ✅ CLA verification complete
5. **PR can now be merged** → (subject to other checks)

## Allowing Bots

Certain bots (dependabot, renovate, etc.) should bypass CLA checks. These are already configured in the workflow:

```yaml
allowlist: dependabot,renovate,bot
```

If you want to add more bots, edit this line in `.github/workflows/cla-check.yml`.

## Troubleshooting

### CLA Assistant Not Appearing on PRs

- Ensure the GitHub App is installed: [CLA Assistant on Marketplace](https://github.com/apps/cla-assistant-lite)
- Verify the repository is added to the app's installation
- Check that branch protection rules include `cla-assistant` as a required check
- Restart the workflow by pushing a new commit to the PR

### Signature File Not Updating

- Ensure the GitHub token has write permissions (check workflow permissions)
- Verify `signatures/cla.json` exists and is in the correct format
- Check the CLA Assistant app logs for errors

### Need to Reset Signatures?

To reset the signature database:

1. Delete the contents of `signatures/cla.json` (keep the structure)
2. Recommit to `main`
3. Re-run the workflow on PRs to re-collect signatures

## More Information

- [CLA Assistant Documentation](https://github.com/cla-assistant/github-action)
- [Contributor License Agreement](../../CLA.md)
- [Contributing Guide](../../CONTRIBUTING.md)

---

If you have questions or issues with CLA setup, please open an issue in the repository.
