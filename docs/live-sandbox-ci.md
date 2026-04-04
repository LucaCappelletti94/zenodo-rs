# Live Sandbox CI

This repository now has two CI layers:

- regular push and pull request checks in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)
- a daily live sandbox workflow in [`.github/workflows/live-daily.yml`](../.github/workflows/live-daily.yml)

The daily workflow is self-contained. Each run creates and publishes its own
tiny sandbox artifacts, then uses those fresh artifacts to exercise the rest of
the API surface.

- feature-matrix builds and tests
- create draft
- update metadata
- reconcile draft files with all supported policies
- delete draft files
- publish
- edit and discard
- create new version
- republish
- record and DOI lookup
- latest-version resolution
- file and archive downloads

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
  The daily workflow creates, edits, versions, and publishes sandbox artifacts,
  so it needs authenticated deposition access and publish actions.

Official references:

- Zenodo developer docs: `https://developers.zenodo.org/`
- Zenodo sandbox notes: `https://developers.zenodo.org/`

## Operational notes

- Never use a production Zenodo token in these workflows.
- The sandbox can be cleaned at any time, but this workflow no longer depends on
  persistent fixture drafts or record DOIs between runs.
- The daily workflow creates new published sandbox artifacts on purpose. Keep
  the uploaded files tiny.
- The live workflow is also exposed through `workflow_dispatch`, so it can be
  run manually after token changes.
