# Live Sandbox CI

This repository now has three CI layers:

- regular push and pull request checks in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)
- a daily live sandbox workflow in [`.github/workflows/live-daily.yml`](../.github/workflows/live-daily.yml)
- a weekly live sandbox workflow in [`.github/workflows/live-weekly.yml`](../.github/workflows/live-weekly.yml)

The daily workflow covers the non-heavy API surface:

- feature-matrix builds and tests
- draft metadata updates
- file reconciliation policies
- draft file deletion
- DOI and record lookup
- file and archive downloads

The weekly workflow covers the heavy persistent flows:

- create draft
- publish
- edit and discard
- create new version
- republish
- latest-version resolution

## GitHub setup

Create a GitHub environment named `zenodo-sandbox` and add the following configuration there.

### Required secret

- `ZENODO_SANDBOX_TOKEN`

What it is:
  A personal access token for `https://sandbox.zenodo.org/`.

How to get it:

1. Register a separate sandbox account at `https://sandbox.zenodo.org/`.
2. Open the Applications page from your sandbox account settings.
3. Create a new personal access token.
4. Grant at least these scopes:
   - `deposit:write`
   - `deposit:actions`

Why it is needed:
  Both the daily and weekly workflows call authenticated deposition endpoints. The weekly workflow also publishes artifacts, which requires `deposit:actions`.

Official references:

- Zenodo developer docs: `https://developers.zenodo.org/`
- Zenodo sandbox notes: `https://developers.zenodo.org/`

### Required variable for the daily workflow

- `ZENODO_SANDBOX_DRAFT_DEPOSITION_ID`

What it is:
  The deposition ID of one dedicated unpublished draft in the Zenodo sandbox.

How to get it:

1. Open `https://sandbox.zenodo.org/` and create a new upload draft.
2. Leave it unpublished.
3. Copy the numeric deposition ID from the draft URL. Zenodo draft pages use URLs like:
   - `https://sandbox.zenodo.org/uploads/<DEPOSITION_ID>`
4. Store only the numeric part as the GitHub variable value.

Why it is needed:
  The daily workflow intentionally avoids publishing. It edits and reconciles files on one managed draft so the job is repeatable and does not create new drafts every day.

Recommendation:
  Use a draft dedicated to CI only. Do not use a human-maintained draft here.

### Required variable for the daily workflow

- `ZENODO_SANDBOX_RECORD_DOI`

What it is:
  The DOI of one tiny published sandbox record used for read-only smoke tests.

How to get it:

1. Publish a tiny sandbox record with at least one small file.
2. Copy the DOI shown on the published record page.
3. Store the DOI string as the GitHub variable value, for example:
   - `10.5072/zenodo.123456`

Why it is needed:
  The daily workflow validates DOI resolution, latest-version resolution, file downloads, and archive downloads without creating new published artifacts every day.

Recommendation:
  Use a tiny text file and keep this fixture record stable.

## Operational notes

- Never use a production Zenodo token in these workflows.
- The sandbox can be cleaned at any time, so if the daily fixture record or draft disappears, recreate it and update the variables.
- The weekly workflow creates new published sandbox artifacts on purpose. Keep the uploaded files tiny.
- Both live workflows are also exposed through `workflow_dispatch`, so they can be run manually after secret or fixture changes.
