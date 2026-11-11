#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Open or update a helm-charts PR for a stable GPU process exporter release.
# Required environment: TAG, GH_TOKEN, APP_SLUG, CHARTS_REPO, CHART_DIR,
# RUNNER_TEMP, GITHUB_REPOSITORY, GITHUB_SERVER_URL, GITHUB_RUN_ID.
set -euo pipefail

: "${TAG:?TAG must be set}"
: "${GH_TOKEN:?GH_TOKEN must be set}"
: "${APP_SLUG:?APP_SLUG must be set}"
: "${CHARTS_REPO:?CHARTS_REPO must be set}"
: "${CHART_DIR:?CHART_DIR must be set}"
: "${RUNNER_TEMP:?RUNNER_TEMP must be set}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}"
: "${GITHUB_SERVER_URL:?GITHUB_SERVER_URL must be set}"
: "${GITHUB_RUN_ID:?GITHUB_RUN_ID must be set}"

if [[ ! "${TAG}" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "::error::expected a stable image tag without a v prefix: ${TAG}"
  exit 1
fi

branch="update/gpu-process-exporter-${TAG}"
work_dir="${RUNNER_TEMP}/helm-charts"

# Create the branch from master, or reset an existing automation branch on retry.
master_sha="$(gh api "repos/${CHARTS_REPO}/git/ref/heads/master" --jq .object.sha)"
if gh api "repos/${CHARTS_REPO}/git/ref/heads/${branch}" > /dev/null 2>&1; then
  gh api -X PATCH "repos/${CHARTS_REPO}/git/refs/heads/${branch}" -f sha="${master_sha}" -F force=true > /dev/null
else
  gh api "repos/${CHARTS_REPO}/git/refs" -f ref="refs/heads/${branch}" -f sha="${master_sha}" > /dev/null
fi

git clone --quiet --depth 1 --branch "${branch}" \
  "https://x-access-token:${GH_TOKEN}@github.com/${CHARTS_REPO}.git" "${work_dir}"
cd "${work_dir}/${CHART_DIR}"

sed -i -E "/^image:/,/^[^ ]/ s/^(  tag:).*/\1 \"${TAG}\"/" values.yaml
grep -Fxq "  tag: \"${TAG}\"" values.yaml

# Replace the final table cell, whether it is empty or already contains a tag.
sed -i -E "/^\| \`image\.tag\`[[:space:]]*\|/ s/\|[^|]*\|[[:space:]]*$/| \`${TAG}\` |/" README.md
grep -Eq "^\| \`image\.tag\`[[:space:]]*\|.*\| \`${TAG}\` \|$" README.md

if git diff --quiet -- values.yaml README.md; then
  echo "gpu-process-exporter image tag and README default already match ${TAG}; no update needed."
  exit 0
fi

current_version="$(sed -n -E 's/^version: *//p' Chart.yaml)"
if [[ ! "${current_version}" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "::error::unexpected chart version format: ${current_version}"
  exit 1
fi
next_version="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.$((10#${BASH_REMATCH[3]} + 1))"
sed -i -E "s/^version: .*/version: ${next_version}/" Chart.yaml
grep -Fxq "version: ${next_version}" Chart.yaml

changed="$(git diff --name-only | sort | tr '\n' ' ')"
expected="${CHART_DIR}/Chart.yaml ${CHART_DIR}/README.md ${CHART_DIR}/values.yaml "
if [[ "${changed}" != "${expected}" ]]; then
  echo "::error::unexpected file set changed: ${changed}"
  exit 1
fi

bot_id="$(gh api "/users/${APP_SLUG}[bot]" --jq .id)"
git config user.name "${APP_SLUG}[bot]"
git config user.email "${bot_id}+${APP_SLUG}[bot]@users.noreply.github.com"
git commit --quiet -am "Update gpu-process-exporter image to ${TAG}"
git push --quiet origin "HEAD:${branch}"

title="Update gpu-process-exporter image to ${TAG}"
body_file="$(mktemp)"
trap 'rm -f "${body_file}"' EXIT
{
  echo "This PR was created automatically by the ${GITHUB_REPOSITORY} release workflow after image ${TAG} was built and published."
  echo
  echo "Changes:"
  echo "- Set \`image.tag\` to \`${TAG}\` in values.yaml and the README.md defaults table"
  echo "- Bumped gpu-process-exporter chart to ${next_version}"
  echo
  echo "Workflow run: ${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}"
} > "${body_file}"

existing="$(gh pr list --repo "${CHARTS_REPO}" --head "${branch}" --state open --json number --jq '.[0].number // empty')"
if [[ -n "${existing}" ]]; then
  gh pr edit "${existing}" --repo "${CHARTS_REPO}" --title "${title}" --body-file "${body_file}"
  echo "updated PR #${existing}"
else
  gh pr create --repo "${CHARTS_REPO}" --base master --head "${branch}" --title "${title}" --body-file "${body_file}"
fi
