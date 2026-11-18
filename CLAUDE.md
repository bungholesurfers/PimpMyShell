# Working on PimpMyShell

Notes for whoever works on this code, people and coding agents alike. What PimpMyShell is: README.md. How it works
inside: ARCHITECTURE.md. This file is the rules that aren't obvious from either.

## Before saying something's done

- `cargo xtask check`: clippy with warnings as errors, every test, the PIMPBOX's own, and the checks below.
- `cargo xtask build` for the release binaries. Everyday builds are quick ones; PGO and LTO are for releases only.
- There's no rustfmt here: follow the code's own style (long lines, comments in plain sentences).

## What people read goes in `lang/`

- Every text a person reads is in `lang/en_US.toml`, never written into the code: use `pms::lang` (see
  `lang/README.md`). Logs and the command line stay English.
- When you change an English text, change its translations in the same change, as best you can, then
  `cargo xtask lang --stamp <code>` for each language you looked over. `cargo xtask lang` lists what's missing, and
  what was translated from English that has changed since.
- `en_RN` (Redneck, Red Hat's own code for it) is ours: keep it current with every change. Write it the way Red Hat's
  1998 translation was: small letters, short, never in a hurry, "yep" and "nawp".
- Trademark notices, licenses and anything with legal weight stay as English says them.
- A text the code compares against (`starts_with`, `==`) must stay a plain string, not a translated one.

## What goes online is on the Network page

- Everything that reaches the network is a `Use` in `pms/src/net.rs` (`USES`), with every program that does it in
  `programs` in `pms-gui/src/copy.rs`, both shown on the settings' Network page. Say whether the SOCKS proxy carries
  it; if it can't, the page has to say so.
- PimpMyShell's own requests go through `pms::fetch` (pms-fetch.exe, a helper): HTTPS only, and the proxy when the settings
  say (`net::route_for`, with the use's SOCKS login), never Windows' proxy settings. The daemon and the settings hold no
  HTTP or TLS library of their own.
- `cargo xtask check` fails when a file that isn't listed in `NETWORK_FILES` (xtask) uses a network API. That's the
  reminder to add the page's entry, not a list to quietly extend.

## Privacy

- Logs leave out the Windows user name, what notifications say, the proxy and the favorites. A new kind of log line
  that says anything about a person goes on the Logs page (`LOG_CONTENTS`).
- No secrets on a command line or in an environment variable: any program of the user's can read them. Hand them over
  a pipe.

## Helper processes

- Work that handles anything from the internet, or a file the user didn't make (WADs, demos, pictures, playlists),
  belongs in a helper process of its own rather than the daemon, so that it can be sandboxed.
- A helper dies with the daemon (a job object), gets only the handles it needs, and has no console.
- A helper starts in efficiency mode unless the settings say not: pms-spawn's `power`, with `pms::efficiency::flags` for its
  part. It isn't inherited, so whatever starts a process says so for it (the PIMPBOX does for its game).

## XAML in Explorer (the TAP)

- Make Grids, StackPanels, Canvases and Paths with `crate::stage::grid()`, `stack_panel()`, `canvas()` and `path()`, never
  `::new()`, and font families with `stage::font()`: Explorer's XAML never frees one made the other way (stage.rs says how
  that was found). `cargo xtask check` fails on one.
- A composition visual once on screen is never freed, closed or not: take sprites and containers from `stage::sprite()` and
  `stage::container()`. `stage::unbuild` gives back what's hung on what it takes apart; `stage::spare` gives back one in a
  layer. Pictures and gradients that come and go are better drawn by XAML (a Border's ImageBrush), as the cards' are; and
  what comes and goes often is better hidden and kept than built again (the Hot Mic alert, `alert::park`).
- Don't build elements that may never go in the tree, with handlers on them: those are never freed either. Take what leaves
  apart with `stage::unbuild`.
- A weak reference to a XAML object made from code dies when nothing of ours holds it, even while the tree still shows the
  object: hold what an asynchronous job will need (toast.rs's FILLING).
- The strings XAML diagnostics hands the TAP (`VisualElement`) are the TAP's to free (xamlom.rs).
- A picture that changes (a `WriteableBitmap` invalidated) costs a frame of XAML for the whole taskbar on its UI thread, ~1–2 M
  cycles, up to the display's refresh rate: what moves at video rate goes on a composition surface instead, presented by the
  process that draws it (screen.rs's `take_surface`). Measure the taskbar thread (the one owning `Shell_TrayWnd`) with
  `QueryThreadCycleTime`.
- A Direct3D device of our own in Explorer costs it ~10 MB and 8 threads, and some of it stays after: don't make one there.

## Telling whether something leaks in Explorer

- Explorer's private bytes are too noisy for this: over 100 cards they swing by several KB a card either way. Count what's in
  use in Explorer's process heap instead, bytes and blocks, from inside the TAP (`HeapWalk` on `GetProcessHeap()`, a few
  tens of ms). Blocks are the steadier figure; a leak shows as a whole number of blocks for each thing made.
- To see what the new blocks are, group them by size and by the module and offset their first word points into: a vtable,
  for an object, which cdb's `ln module+offset` names (Windows' public symbols cover XAML and DirectComposition).
- Make it fast: send hundreds of cards (or whatever's measured) from a thread inside the TAP, a few ms apart, rather than one
  `pms` command at a time; restart Explorer first, for a small heap and a clean start. Then a thousand takes half a minute.
- A snapshot of the heap kept inside Explorer is itself in the heap: take it before the first reading, not after.
- After changing how something's drawn, look at it on screen. A bench of pictures that were never drawn measured no leak.
- Such tools are for the hunt: take them out of the TAP when it's over.

## Timing things

- The GPU's clocks come and go: drivers drop to a low power state between bursts of work, so anything that waits on the GPU
  (a copy, a fence, a readback) can take several times longer when it's lightly loaded than when it's busy. Wall-clock times
  around GPU work are rough; compare runs made under the same load, never one quiet run against one busy one.
- For what the CPU spends, count the thread's own cycles (`QueryThreadCycleTime`): waiting on the GPU costs none, so the
  GPU's clocks barely move them.

## Trying a dev build

- On a scratch copy of the settings, never on `%APPDATA%\PimpMyShell`: a dev build's migrations are one-way.
- Never `pms inject` while a daemon runs.
- Every daemon (or `pms inject`) of a new build loads another copy of the TAP into Explorer, about 2–2.5 MB, which Windows
  can't unload; a restart on the same build reuses the copy there. After a day of rebuilds, restart Explorer.
- Explorer is restarted only through PimpMyShell's own restart, which refuses while a folder window or a file copy is
  open.

## Files and git

- Delete or overwrite only PimpMyShell's own files, by their exact names; nothing recursive on a folder that could
  hold the user's things.
- Only `main` and `v*` tags go to GitHub. A `v*` tag starts the release workflow, which makes a draft release.
