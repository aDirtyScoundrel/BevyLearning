#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
One-command release workflow for Learning.

Usage:
  ./scripts/release.sh --version v0.1.1 [--upload-release] [--dry-run]

Options:
  --version <tag>     Release tag/version (example: v0.1.1)
  --upload-release    Upload package as GitHub Release asset (requires GITHUB_TOKEN)
  --dry-run           Print actions without changing git/GitHub state
  -h, --help          Show this help

What this script does:
  1. cargo build --release
  2. package target/release/learning into dist/learning-linux-x86_64-<version>.tar.gz
  3. commit packaged artifact (if there are staged changes from this step)
  4. create/push git tag
  5. push main branch
  6. optionally create/update GitHub Release and upload asset
EOF
}

VERSION=""
UPLOAD_RELEASE=0
DRY_RUN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --upload-release)
      UPLOAD_RELEASE=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  echo "Missing required --version" >&2
  usage
  exit 2
fi

if [[ ! "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9]+)?$ ]]; then
  echo "Version must look like vMAJOR.MINOR.PATCH (or pre-release suffix), got: $VERSION" >&2
  exit 2
fi

run_cmd() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[dry-run] $*"
  else
    eval "$@"
  fi
}

echo "==> Releasing $VERSION"

if [[ "$UPLOAD_RELEASE" -eq 1 && -z "${GITHUB_TOKEN:-}" ]]; then
  echo "--upload-release requires GITHUB_TOKEN in the environment" >&2
  exit 3
fi

if [[ "$(git branch --show-current)" != "main" ]]; then
  echo "Release script expects to run from main branch" >&2
  exit 1
fi

run_cmd "cargo build --release"

ASSET_DIR="dist"
ASSET_NAME="learning-linux-x86_64-${VERSION}.tar.gz"
ASSET_PATH="${ASSET_DIR}/${ASSET_NAME}"
run_cmd "mkdir -p ${ASSET_DIR}"
run_cmd "tar -czf ${ASSET_PATH} -C target/release learning"

run_cmd "git add ${ASSET_PATH}"

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[dry-run] git commit -m \"Ship package ${VERSION}\" (if needed)"
else
  if ! git diff --cached --quiet; then
    git commit -m "Ship package ${VERSION}"
  else
    echo "No staged changes to commit"
  fi
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[dry-run] create/push tag ${VERSION} if missing"
else
  if ! git rev-parse -q --verify "refs/tags/${VERSION}" >/dev/null; then
    git tag -a "${VERSION}" -m "Release ${VERSION}"
  else
    echo "Tag ${VERSION} already exists locally"
  fi
fi

run_cmd "git push origin main"
run_cmd "git push origin ${VERSION}"

if [[ "$UPLOAD_RELEASE" -eq 1 ]]; then
  REMOTE_URL="$(git remote get-url origin)"
  REPO_PATH="${REMOTE_URL#https://github.com/}"
  REPO_PATH="${REPO_PATH%.git}"

  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[dry-run] create/update GitHub release ${VERSION} and upload ${ASSET_NAME}"
  else
    curl -sS -X POST \
      -H "Accept: application/vnd.github+json" \
      -H "Authorization: Bearer ${GITHUB_TOKEN}" \
      "https://api.github.com/repos/${REPO_PATH}/releases" \
      -d "{\"tag_name\":\"${VERSION}\",\"name\":\"${VERSION}\",\"generate_release_notes\":true}" >/dev/null || true

    RELEASE_JSON="$(curl -sS \
      -H "Accept: application/vnd.github+json" \
      -H "Authorization: Bearer ${GITHUB_TOKEN}" \
      "https://api.github.com/repos/${REPO_PATH}/releases/tags/${VERSION}" | tr -d '\n')"

    UPLOAD_URL="$(echo "$RELEASE_JSON" | sed -n 's/.*"upload_url":"\([^"]*\)".*/\1/p' | sed 's/{?name,label}//' | sed 's#\\/#/#g')"
    HTML_URL="$(echo "$RELEASE_JSON" | sed -n 's/.*"html_url":"\([^"]*\)".*/\1/p' | sed 's#\\/#/#g')"

    if [[ -z "$UPLOAD_URL" ]]; then
      echo "Failed to parse release upload URL" >&2
      echo "$RELEASE_JSON" | head -c 500 >&2
      exit 4
    fi

    UPLOAD_RESP="$(curl -sS -X POST \
      -H "Accept: application/vnd.github+json" \
      -H "Authorization: Bearer ${GITHUB_TOKEN}" \
      -H "Content-Type: application/gzip" \
      --data-binary @"${ASSET_PATH}" \
      "${UPLOAD_URL}?name=${ASSET_NAME}")"

    if echo "$UPLOAD_RESP" | grep -q '"state":"uploaded"'; then
      echo "Uploaded release asset: ${ASSET_NAME}"
    elif echo "$UPLOAD_RESP" | grep -q '"already_exists"'; then
      echo "Release asset already exists: ${ASSET_NAME}"
    else
      echo "Unexpected upload response:" >&2
      echo "$UPLOAD_RESP" | head -c 700 >&2
      exit 5
    fi

    echo "Release URL: ${HTML_URL}"
  fi
fi

echo "==> Done: ${VERSION}"
