#!/usr/bin/env bash
# shellcheck shell=bash
# Classify a release version into its GitHub Release channel (R0.14).
#
# Usage: packaging/release-channel.sh <version> [alpha-input]
#
#   <version>      SemVer release version, with or without a leading `v`
#                  (for example `0.1.0-rc.1` or `v1.0.0`).
#   [alpha-input]  the Release workflow's `alpha` input (`true` marks a stable
#                  version as an Alpha prerelease; empty on tag pushes).
#
# Prints GITHUB_OUTPUT-style lines:
#   prerelease=true|false
#   title=CurveBook [Alpha|Beta|Release Candidate|Prerelease ]v<version>
#
# Any SemVer pre-release version (`-alpha.N`, `-beta.N`, `-rc.N`, ...) is a
# GitHub prerelease, so a tag push can never publish a candidate as "Latest".
# Build metadata (`+...`) is ignored when classifying.
set -euo pipefail

ver="${1:-}"
alpha_input="${2:-}"
if [[ -z "$ver" ]]; then
  echo "usage: release-channel.sh <version> [alpha-input]" >&2
  exit 2
fi
ver="${ver#v}"

semver_re='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'
if [[ ! "$ver" =~ $semver_re ]]; then
  echo "error: release version '$ver' is not valid SemVer" >&2
  exit 1
fi

core="${ver%%+*}"
pre=""
if [[ "$core" == *-* ]]; then
  pre="${core#*-}"
fi

label=""
prerelease=false
if [[ -n "$pre" ]]; then
  prerelease=true
  first="$(printf '%s' "${pre%%.*}" | tr '[:upper:]' '[:lower:]')"
  case "$first" in
    alpha*) label="Alpha" ;;
    beta*) label="Beta" ;;
    rc*) label="Release Candidate" ;;
    *) label="Prerelease" ;;
  esac
elif [[ "$alpha_input" == "true" ]]; then
  prerelease=true
  label="Alpha"
fi

title="CurveBook v$ver"
if [[ -n "$label" ]]; then
  title="CurveBook $label v$ver"
fi

printf 'prerelease=%s\n' "$prerelease"
printf 'title=%s\n' "$title"
