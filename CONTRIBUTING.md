# Contributing to FrameMate

Patches, bug reports and testing on other SteamOS versions are all welcome.
FrameMate leans on undocumented Steam internals, so reports of "this broke
after an update" are useful even without a fix attached.

## Sign your commits (DCO)

FrameMate uses the [Developer Certificate of Origin](DCO), same as the Linux kernel. 
There is no CLA to sign and no copyright assignment: you keep the copyright on your work.

Every commit must carry a `Signed-off-by` line matching the author:

```sh
git commit -s -m "Fix controller battery after sleep"
```

That appends:

```
Signed-off-by: Your Name <your.email@example.com>
```

Forgot it? `git commit --amend -s` for the last commit, or
`git rebase --signoff main` for a branch. Please use a name you're comfortable having in the public git history, sign-offs are permanent.

By signing off you certify the four points in [`DCO`](DCO), and that your
contribution is offered under:

- the **GNU GPL v3.0 or later** ([`LICENSE`](LICENSE)); **and**
- the additional terms in [`LICENSE-EXCEPTION.md`](LICENSE-EXCEPTION.md) (notably the GPL §7 app store distribution permission).

That second point is key, it allows future app store releases without needing to get approval from every past contributor. 

## Project layout

| Path | |
|---|---|
| `crates/agent` | The agent that runs on the Frame. Static `aarch64-unknown-linux-musl` binary, shipped as a Flatpak |
| `app` | The phone app: SvelteKit frontend |
| `app/src-tauri` | The app's Rust shell |
| `scripts` | `flatpak.sh` (build/install the agent), `deploy.sh` (dev loop on the Frame), `release.sh` (tag a release) |

Following should be preserved:

- **The agent stays small and dependency-light.** It's ~6.5 MB and idles at
  practically zero CPU on a battery-powered headset. New dependencies in
  `crates/agent` need to earn their place.
- **The agent stays free of heavyweight media stacks.** Encoding talks to the
  V4L2 encoder through raw ioctls (`crates/agent/src/{v4l2,encoder}.rs`)
  because GStreamer's `v4l2h264enc` fails caps negotiation with this driver and isn't in the Flatpak runtime.

## Development

Setup and build commands are in [Building from
source](README.md#building-from-source). The quick loop:

```sh
scripts/deploy.sh          # build + run the current agent on the Frame (| logs | stop)
cd app && deno task tauri dev # desktop window for UI work
```

`scripts/*.sh` reach the Frame as `steamos@frame.local`; override with
`FRAME_HOST`.

## Before you open a PR

CI only builds release artifacts on pushed `v*` tags, which needs push access to this repository, so it never sees a pull request. Nothing will catch mistakes for you, so please run these yourself:

```sh
cargo test                   # agent unit tests (host target, not musl)
cargo clippy --all-targets
cd app && deno task check   # svelte-check + TypeScript
```

Also:

- **Don't bump version numbers manually.** `scripts/release.sh` sets them everywhere at
  once, and the release job verifies the tag matches `Cargo.toml` and
  `tauri.conf.json`.
- Keep commits focused, and match the surrounding comment style
- Document *why* a hardware or Steam quirk is handled a certain way
- If a change depends on a particular SteamOS or Steam client version, say so in the PR.

## Reporting bugs

Use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.md) and include
your SteamOS version (Settings -> System), the agent version, and relevant agent
logs from `scripts/deploy.sh logs`.
