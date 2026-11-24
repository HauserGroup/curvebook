# Security Policy

## Supported versions

CurveBook is currently in prerelease (`0.1.0-alpha.*` / Beta preparation; [`v0.1.0-alpha.1`](https://github.com/HauserGroup/curvebook/releases/tag/v0.1.0-alpha.1) published 2026-11-17). Security fixes go to the `main` branch only and ship in the next release; earlier prerelease tags do not receive backports.

## Reporting a vulnerability

**Do not open a public issue, discussion, or pull request for a security problem.**

Report it privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. If that option is not shown, contact a maintainer listed in [`.github/CODEOWNERS`](.github/CODEOWNERS) privately and ask how to send the details.

Please include:

- what the problem is and what an attacker could do with it,
- the commit or build you tested, and the platform (macOS, Windows, Linux, FreeBSD, or web),
- the smallest input or sequence of steps that triggers it,
- whether it is already public anywhere.

Do not include unpublished research data or other confidential material in a report. A synthetic file that triggers the problem is enough.

## What to expect

A maintainer will confirm that the report arrived, assess it, and tell you what happens next. Please give us reasonable time to fix the problem before you disclose it. We will credit you in the fix unless you ask us not to.

## What counts

CurveBook opens files and accepts requests that it must treat as hostile. These are in scope:

- a project archive (`.curvebook`, `.cbtemplate`, `.prism`, `.pzfx`), CSV, or pasted table that causes memory exhaustion, unbounded decompression, a hang, or writes outside the intended location,
- a user-defined equation or formula that escapes its evaluator or exhausts resources,
- a request over the MCP server, the CLI, or the `--control` channel that reads or writes files, or runs code, beyond what the request is meant to allow,
- the `--control` channel being reachable from anywhere other than the local machine,
- a problem in the release or packaging pipeline that could let someone alter a published build.

A crash on malformed input is a bug we want to hear about, because the project's rule is that shipped code never panics. Unless it can be turned into one of the problems above, report it as an ordinary bug with the bug report issue template.

Vulnerabilities in a dependency should go to that dependency's maintainers first. Tell us as well if CurveBook is affected in a way that needs a change here.
