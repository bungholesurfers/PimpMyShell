# How PimpMyShell diddles Explorer

How it works inside, for whoever wants to check what it does, or change it. What it is, what it changes on your PC and how
to get it are in the [README](README.md).


The Windows 11 taskbar is a system XAML (`Windows.UI.Xaml`) island hosted inside explorer.exe. PimpMyShell gets a DLL
into that process through the front door Microsoft built for Visual Studio's Live Visual Tree, and then edits the
taskbar's element tree from inside.

```
 pmsd.exe (daemon, your session)                         explorer.exe
 ├─ config, hot reload                                   └─ pms_tap.dll (the "TAP")
 ├─ PDH / DXGI / NVML sampler    \\.\pipe\pms-tap-N         ├─ IVisualTreeService3 (mirror of the taskbar tree)
 ├─ mic watch (consent store)  ─── length-prefixed ───▶     ├─ command interpreter on the pipe thread
 ├─ toast listener + banners      text commands             └─ XAML / Composition work on the UI thread
 ├─ watchdog (exit code, event log)
 └─ tray icon
```

## The crates

| Crate | What it is |
| --- | --- |
| `pms` | the daemon (`pmsd.exe`, windowless; `pms.exe daemon` in a console), the `pms` command line, config, sampler, watchdog, tray, updater |
| `pms-tap` | `pms_tap.dll`, the TAP loaded into explorer. Draws everything in the taskbar and serves the pipe |
| `pms-xaml` | a `windows-bindgen` projection of the slice of `Windows.UI.Xaml` it needs. windows-rs stopped shipping XAML, but its metadata still describes it |
| `pms-gui` | the settings: WinUI 3 (windows-reactor) where the Windows App Runtime is installed, else egui, over one model; and `--ask`, the daemon's questions as WinUI 3 windows |
| `pms-twatch` | `pms-twatch.exe`, Twatch TV's player: Windows' own adaptive player, each frame averaged down on the GPU and presented to a DirectComposition surface, which the TAP shows under the set's picture (the daemon offers it the surface's handle through the screen's shared memory; the TAP takes it from the player only if it's pms-twatch.exe, and only if it's a surface), with no copy and nothing on Explorer's UI thread for any frame. Until then, or where it can't be, frames go through the screen as pixels. With a proxy, every request Windows' player makes goes through an `IHttpFilter` of ours, so Windows' HTTP stack never sends one |
| `pms-fetch` | `pms-fetch.exe`, every request the daemon and the settings make (Twitch's pictures, decoded there, playlists, downloads, the updater's, the ISS's telemetry for TV channel 37, Test), with win32k off when it's sandboxed; neither of them holds a TLS library. Two run when some uses go through the proxy and some don't, so the ones that don't never open a way around it. Sandboxed (LPAC) with a proxy, it gets each connection as a socket the daemon connected to the proxy and does the SOCKS itself. `pms-fetch.exe relay` is the relay in the other helpers' containers, their only way to a proxy on 127.0.0.1 |
| `pms-pimpbox` | its own workspace, GPL: `pms-pimpbox.exe`, the console (its window, its boot animation in `boot`, MIT); `pms-poom.exe` (`poom`), the game, on `room` (`engine`), with no window and nothing of user32 or gdi32, so that sandboxed it starts with win32k off, and `pms-poom --check`, which plays a speed demo unseen and says when it left its level (the daemon's demos.rs keeps only those that leave at the archive's time, and hands the console a few for the title loop, `-demos`); and `poom`'s library, what the two say to each other (pictures in shared memory one way, the window's keys and mouse down a pipe the other) |
| `pms-spawn` | starts every helper: a job that dies with the daemon, only the handles it's handed, mitigations, and its AppContainer if it has one |
| `pms-ui` | widgets the settings and the art tool share (the knobs) |
| `pms-art` | renders the icons and the installer's pictures from `art\` |
| `pms-res` | puts the icon and version info into the exes |
| `pms-recon` | step zero: dumps the HWND tree of every taskbar, to see what hosts what (system XAML vs. WinUI 3) |
| `xtask` | the build tasks |

## Getting in: `InitializeXamlDiagnosticsEx`

`Windows.UI.Xaml.dll` exports `InitializeXamlDiagnosticsEx(endpoint, pid, dll, tapDll, tapClsid, initData)`. It's the
XAML Diagnostics entry point: given a process id and a DLL, XAML in *that* process loads the DLL, creates a COM object
from it by CLSID, and hands it an `IXamlDiagnostics` site. That DLL is a TAP ("Tool/Test Attach Point").

`pms inject` (and the daemon) does this:

1. Finds explorer's pid from `Shell_TrayWnd`, but only once it has its
   `Windows.UI.Composition.DesktopWindowContentBridge` child, i.e. the XAML island exists.
2. Asks any TAP already there to `detach`. One from 0.1.7 or before answers to the older name `\\.\pipe\pms-tap`, so
   that's tried as well.
3. Copies `pms_tap.dll` to `%LOCALAPPDATA%\PimpMyShell\tap\pms_tap-<timestamp>.dll`. Explorer keeps every copy it loaded
   locked until it exits, so each injection gets a fresh copy. Old copies get swept up when they're no longer loaded.
4. Calls `InitializeXamlDiagnosticsEx` with that copy and the TAP's CLSID, trying endpoint names
   `VisualDiagConnection1..100` until one isn't taken.
5. Waits for the new TAP to open its pipe and answer `ping`.

## Inside: the TAP

The TAP is a plain COM object implementing `IObjectWithSite` and `IVisualTreeServiceCallback2`, exported through
`DllGetClassObject`. The XAML Diagnostics interfaces (`xamlom.h`) aren't in windows-rs, so they're hand-written in
`pms-tap/src/xamlom.rs`, vtable order and all.

- **`SetSite`** receives the site and casts it to `IVisualTreeService3` and `IXamlDiagnostics`.
- **`AdviseVisualTreeChange`** replays the whole existing tree through `OnVisualTreeChange` and then reports every
  add and remove. It must be called from another thread: called from `SetSite` (on the UI thread) it deadlocks.
- The TAP keeps a **mirror** of the tree: handle, type, name, parent, child index. Commands find elements in it by
  name or type (`BackgroundFill`, `TaskListButton:2`), and it's dumped to `tree.txt` when it changes.
- **`GetDispatcher`** gives the island's `CoreDispatcher`. Anything touching XAML is posted to the UI thread with
  `RunAsync`, waited on for at most 5 seconds, and wrapped in `catch_unwind`: a Rust panic unwinding into XAML would
  take explorer with it.
- `DllCanUnloadNow` always says no. The TAP leaves threads running, so COM must never unload it. A detached TAP calls
  `UnadviseVisualTreeChange` and goes inert.

The diagnostics API can also read the property chain (`GetPropertyValuesChain`), create objects from strings
(`CreateInstance`), and `SetProperty` / `ClearProperty` / `AddChild` / `RemoveChild`. For real drawing, the TAP gets
the live object behind a handle (`GetIInspectableFromHandle`) and uses the generated `pms-xaml` bindings like any XAML
app would.

## The layer cake

On the builds PimpMyShell was made on, the taskbar's background looks like this:

```
TaskbarFrame
└─ RootGrid
   └─ BackgroundControl
      └─ Grid
         ├─ BackgroundFill        Rectangle, Fill = the stock AcrylicBrush (blur, tint and noise in one effect)
         └─ BackgroundStroke
   (the buttons live in a later sibling, so anything in this Grid sits under them)
```

The TAP inserts its own elements into that Grid, right after `BackgroundFill`, always in this order:

```
BackgroundFill, [Backdrop host], [Alert host], [Stage canvas], [Grain host], BackgroundStroke, ...
```

- **Backdrop.** The stock `AcrylicBrush` bakes blur, tint and grain into one effect, so nothing can go in between.
  With `[backdrop]` on, the TAP parks the stock brush (keeping a strong reference, to put it back later), sets
  `BackgroundFill.Fill` to a transparent brush, and hands its own composition tree to an empty Grid with
  `ElementCompositionPreview.SetElementChildVisual`:
  - a `SpriteVisual` with a `HostBackdropBrush` (DWM's already-blurred desktop),
  - a tint `SpriteVisual`,
  - a "layer" `SpriteVisual` for washes and pulses,
  - a container that paints the free gaps between buttons (it follows layout changes),
  - and, in the Grain slot further up, tiles of noise: a 512x128 grayscale PNG generated at startup, loaded through
    `LoadedImageSurface`. The grain knob is that visual's opacity.

  DWM only feeds a host backdrop to windows that opted in, and only takes the opt-in from the process that owns the
  window. So the TAP itself sets `DWMWA_USE_HOSTBACKDROPBRUSH` on explorer's `Shell_TrayWnd` and
  `Shell_SecondaryTrayWnd` windows.
- **Alert slot.** The hot-mic alert: a `ShapeVisual` with a breathing rounded-rect wash and sonar-ring ellipses. The
  toast ripple and the button glows sit here too.
- **Stage.** A hit-test-invisible `Canvas` holding the meter strip. Above the blur and the alert, below the grain, so
  meters get grained like the rest of the taskbar.

Because the pointer never reaches the stage, meter tooltips use transparent hover zones placed in the top-level Grid
(the one holding `TaskbarFrame` and `SystemTrayFrame`), and a windowed `Popup` that can stand above the taskbar.
Toast cards go in that Grid too, over the tray (the cards behind the front one peek out at its left). The tray fades
away while they show.

Where things fit comes from **layout tracking**: on every `LayoutUpdated` the TAP measures the children of
`TaskbarFrameRepeater` and `SystemTrayFrameGrid` and works out the free stretches. The meter strip fits itself into
the space beside the tray. When it runs short, it squeezes spacing, then graphics, then text width, then hides meters
from the end of the list.

**Glows.** The Start button (`LaunchListButton`) is a `ToggleButton`: the TAP listens to its `IsPointerOver`,
`IsPressed` and `IsChecked` (Start menu open) with `RegisterPropertyChangedCallback`. App buttons (`TaskListButton`)
take pointer events instead. Each app's color comes from rendering its icon with `RenderTargetBitmap` and picking the
most prominent hue.

**Frame pacing.** The compositor runs keyframe animations at the display's rate and has no setting for a slower one.
So `pace <fps>` samples a long-running animation into held steps, and nothing on screen changes between them.

## The pipe

The TAP serves `\\.\pipe\pms-tap-N`, where N is the sign-in session's number. Pipes' names are the whole PC's, and
everyone signed in has an Explorer of their own, so each gets a pipe of their own.

- one instance, made once and kept for as long as the TAP serves (`FILE_FLAG_FIRST_PIPE_INSTANCE`): a stale TAP can't
  share it, and nothing else can take the name while a TAP has it. Remote clients are rejected,
- a DACL of SYSTEM and the owner only, so nobody else can give the TAP commands,
- each request and each response a frame: a little-endian `u32` length, then UTF-8 text,
- a request is a command line: `status`, `reset`, `backdrop tint #7F060606`, `meters push ...`, `toast show ...`.
  `batch "<cmd>" "<cmd>" ...` runs several in one UI-thread job, one layout pass.

The other end is as careful, because a pipe's name belongs to whoever makes it first, and what goes down this one is
your notifications' text. Before it sends a word, the daemon (and `pms`) checks that the pipe is Explorer's:

- its owner is you,
- it has no low-integrity label. Windows puts one on whatever a sandboxed, low-integrity process makes, and won't let
  such a process claim better,
- the process serving it is the one the taskbar's in.

It connects with `SECURITY_IDENTIFICATION` too, so that whatever answers can't act as you. A pipe that fails the check
gets nothing sent to it, and nothing it says is acted on. The daemon says so in its log and leaves the taskbar alone,
and if the pipe's still there ten seconds later it stops and tells you, as it does for an Explorer that keeps crashing.

Up to 0.1.7 the name was `pms-tap`, the same for everyone. From 0.1.8, injecting tells a TAP under that name to leave
as well, which is how an update takes over from the TAP that's there. It doesn't work the other way: a 0.1.7 daemon
can't see a TAP from 0.1.8, and brings its own. Stop PimpMyShell before going back to 0.1.7, which the installer does
for you, and the newer TAP has taken its meters away by then. If its daemon was ended from Task Manager instead, both
TAPs draw until Explorer restarts.

The daemon opens a fresh connection per request, so nothing hogs the single instance. `pms` with no arguments is a
REPL against the same pipe, and `pms help` lists the commands, including raw tree surgery like
`pms props BackgroundFill/Fill` and `pms set <elem> <Prop> <value>`.

## The daemon's loop

`pmsd.exe` holds a single-instance mutex and sleeps in `WaitForMultipleObjects` on a stop event, the mic watcher's
registry event and the tray's event, waking for whichever is due: meter intervals, mic levels, toast polls. Once a
second it reconciles:

1. **`status`.** The TAP answers `tap=<copy id> build=<hash> taskbar=<BackgroundFill handle> backdrop=.. alert=..
   meters=N toast=..`. The UI-thread hop doubles as a liveness check.
   - No pipe at all (first run, explorer restarted): **inject**, once the taskbar's island exists, at most every
     5 seconds.
   - A pipe that isn't Explorer's (see The pipe): nothing's sent to it. Ten seconds of that, and the daemon stops and
     says so.
   - Three failed exchanges with a TAP that's there: replace it.
   - A TAP of another build (`build` is a hash of the TAP DLL's bytes; the daemon hashes the `pms_tap.dll` beside it):
     **swap ours in**. After an update explorer still has the TAP it loaded before, which may not speak the new
     daemon's protocol, so toasts would fail while their native banners are off. Swapping doesn't count against
     the hook, and every update takes effect without restarting explorer. If two swaps in a row don't take, that TAP
     keeps the rest of the config but not the toasts: native banners stay on until explorer restarts.
   - A TAP copy or taskbar tree it hasn't configured, or one that drifted from the config (someone ran `reset`):
     **`reset`**, then apply the whole config again.
2. **Config hot reload.** When `pms.toml`'s modified time changes, it's loaded and validated. Only the sections that
   changed are re-sent, in one batch, so saving from the settings doesn't flash the taskbar back to stock. A broken
   file keeps the previous config, and a notice (`notices.rs`) says what's wrong. One broken at the start doesn't stop
   the daemon: it starts with a stock taskbar, until a save that loads.
3. **Theme and power.** It follows `SystemUsesLightTheme` (with `theme = "auto"`) and switches between the dark and
   light looks. On battery (`GetSystemPowerStatus`) it lowers the animation cap.
4. Then the hot mic, toasts and meters (below).

Exit, `pms daemon stop`, or Ctrl+C sends `reset` (stock taskbar) and restores the banners. The TAP stays loaded,
inert, until the next daemon.

## Where the meter numbers come from

- **PDH.** Counters are added with `PdhAddEnglishCounterW`, so configs use English paths whatever the display
  language. Wildcard instances (`\Network Interface(*)\Bytes Received/sec`) are aggregated with `sum`, `max`, `min`,
  `avg`, or `engine` (per-engine sum, then the busiest engine, the way Task Manager computes GPU %), skipping `_Total`.
  Meters that share an interval share a PDH query, collected only when that interval comes round. `typeperf -q`
  lists the paths.
- **Expressions.** A tiny parser: `+ - * /`, parentheses, `min`, `max`, `abs`, over counter names and builtins
  (`ram_total`, `vram_total`, `swap_total`, `swap_used`, `cpu_count`).
- **CPU threads.** Per-CPU busy time from `QueryIdleProcessorCycleTimeEx` against the TSC. The user/privileged time
  counters only move in whole timer ticks, so they just decide the user/kernel split. Grouping comes from the CPU topology
  Windows reports.
- **GPUs.** DXGI enumerates the adapters (software ones like WARP left out unless asked). `per_gpu = true` turns one
  meter into a pie per adapter with at least `min_memory_mb` (1024) of dedicated memory, reading `\GPU Adapter Memory(luid_..._phys_*)\Dedicated Usage`
  against the adapter's size. The maker comes from the adapter's name. An engine counts as working from 0.5% (the
  desktop keeps some at a trace); with `engines_fall_off`, idle engines leave one by one, not only whole GPUs.
- **Disks and network interfaces.** `\PhysicalDisk(*)` and `\Network Interface(*)` per instance. A disk's instance
  is its number and drive letters (`0 C: D:`); the one holding the Windows folder is the system disk. Its model and
  bus come from `IOCTL_STORAGE_QUERY_PROPERTY` (no access to the disk itself needed). An interface's instance is
  its adapter's description with `(` `)` `#` `/` `\` respelled, matched against `GetAdaptersAddresses` for its name,
  kind, link speed and addresses. The internet one is where `GetBestInterfaceEx` routes 1.1.1.1 (IPv6: Cloudflare's
  2606:4700:4700::1111), which asks the routing table and sends nothing. That one and the system disk always show,
  first. The rest show while busy (2 KB/s or more), busiest first, and leave `hold_seconds` after going idle. The
  order is worked out again only when one comes or goes, so bars don't swap under the pointer.
- **Tooltips.** A hover zone covers each meter at the taskbar's full height and reaches into the spacing around it.
  GPU and device meters get one per GPU or device. The daemon writes those tooltips: a GPU's name, power state and
  busy engines; a disk's role, rates, model and free space; an interface's role, rates, adapter, link speed and
  addresses.
- **NVML.** If NVIDIA's driver is installed, `nvml.dll` is loaded at runtime for each GPU's performance state. No
  NVIDIA, no NVML, no problem.

## Hot mic

The same source as Windows' own mic indicator: the capability consent store,
`HKCU\...\CapabilityAccessManager\ConsentStore\microphone`. Every app that ever used the mic has a key there (packaged
apps by family name, others under `NonPackaged` by path) with `LastUsedTimeStart` and `LastUsedTimeStop`. While an app
holds the mic, `LastUsedTimeStop` is 0. `RegNotifyChangeKeyValue` wakes the daemon on any change, so the warning
shows up at once.

App names come from the exe's `FileDescription` or the package's display name. While the mic is hot, the daemon reads
every active capture endpoint's `IAudioMeterInformation` peak about 15 times a second, plus the default endpoint's mute,
and feeds them to the alert. Apps that mute themselves (Discord, Teams) still read as live: Windows can't see that.

## Device names

The taskbar names devices the way Windows does inside. A network interface goes by its LUID's name, the closest thing
Windows has to Linux's `eth0`: `ethernet_32773` (what `Get-NetAdapter` calls `InterfaceName`) shows as `eth5`, and
`wireless_32768` as `wlan0`, counting from the 32768 that Windows starts at. A disk goes by its device name:
`\\.\PhysicalDrive0` shows as `disk0`, the number diskpart and Disk Management use. The tooltips give the full names.

## Toasts

- **Source.** `UserNotificationListener` works unpackaged once you allow notification access. Its change event needs
  package identity, so the daemon polls, but not the listener: twice a second it reads, from the notification platform's
  `wpndatabase.db`, the newest notification's id and how many there are, a read of a file that costs Windows nothing, and asks
  the listener only when those change (and every 30 s, in case). Asking it has WpnUserService gather every notification in the
  center each time: about 57 ms of its CPU with 260 of them, and on some PCs a stream of writes to that database's log
  (`wpndatabase.db-wal`). The listener only exposes text, so the full toast XML
  (images, buttons, launch arguments) is read from the notification platform's `wpndatabase.db` by the same id,
  read-only. Without access (or once it's taken away), no banner is turned off, a notice says so, and the daemon
  tries again every 15 seconds.
- **How long a card stays.** `duration_ms`, or at most 4 seconds when a card comes forward with more behind it.
  `duration="long"` gets at least 25 seconds. The `alarm` and `incomingCall` scenarios, and `reminder` with a button
  (Windows ignores the scenario without one), have no time limit, as in Windows: they stay until you answer them, or
  until the app takes the notification back. `urgent` only gets past do not disturb, so it times out like any other.
- **When the taskbar won't take a toast.** With native banners off it would go missing. One refusal may be a glitch or
  one app's odd toast, but two in a row give every banner back to Windows, and a notice says so, as a Windows
  notification. The taskbar gets another try after 5 minutes, then after twice as long each time it refuses, up to an
  hour; the first toast it shows ends all that. Cards and meter tooltips go in the nearest Grid above the tray, which is normally its parent; if not, the
  TAP's log names what's between them.
- **When something keeps failing.** Three failures in a row to set up or feed the meters raise a notice asking for a
  report: your Windows version, the logs and settings, and the taskbar's layout, redacted like the logs. *Save a report* in
  the tray menu makes the same file any time. Nothing is sent: the report is saved beside the logs, and a window explains
  that GitHub issues are public before offering to open GitHub.
- **Reports.** The daemon's log starts with the Windows version, then, for each explorer, the versions of what we hook
  (`explorer.exe`, `Taskbar.dll`, `Taskbar.View.dll`, `Windows.UI.Xaml.dll`), the taskbar software it knows, and every
  third-party DLL in explorer. File names and versions only, not paths. Both logs leave out your Windows user name
  (paths show `%USERPROFILE%` or `<user>` instead) and what your notifications say: a toast is logged by its app's name
  alone, and a link it opened by nothing more than its scheme.
- **Native banners out of the way.** `ShowBanner=0` under an app's `Notifications\Settings\<AUMID>` key takes effect
  live: the notification still reaches the center (and the listener), but the popup is gone. Set ahead of an app's
  first notification, it holds that one back too. So the daemon sets it for every app that has notified, and also for
  every app that could: the Start menu's apps (packaged ones, and shortcuts carrying an app id, like Telegram's or
  Discord's) plus the ids registered under `Classes\AppUserModelId` (Firefox's, Windows' own senders). It looks
  again every 10 minutes for newly installed apps. Restoring is as the [README](README.md#is-it-safe-what-does-it-change) describes.
- **Every kind goes through one platform.** Modern toasts (WinRT, Windows App SDK, browsers' web push), Windows'
  own system toasts, and the old tray balloons (`Shell_NotifyIcon` with `NIF_INFO`, and `IUserNotification`),
  which Explorer turns into toasts. All of them show up in the listener and go to the taskbar. Try them all with
  `cargo run -p pms --example toast_lab -- register` once, then `-- everything` (or `all`, `shell`, or kinds by
  name; `--gap 4` first to space them out); `notif_probe` shows what the listener and the database hold.
- **Two exceptions.** A tray icon's balloons get an id Explorer makes up (`NotifyIconGeneratedAumid_<hash>`) at
  its first balloon, so each tray icon's first balloon still pops natively. And a toast an app sends silently
  (`SuppressPopup`, straight to the center) looks like any other to the listener, so it shows in the taskbar.
- **A deck.** A toast that comes while others show goes on top. The ones behind step back: shorter, fainter, their
  insides hidden, and clipped to a sliver that peeks out at the front card's left (the cards are translucent, so
  anything more would show through). When the front card goes, the next comes forward, with at most 4 s while more
  wait behind it. Up to 8 cards; older ones are let go (they're still in the notification center).
- **Swiping.** Drag the front card: it follows the pointer, fading as it goes right, and gives a little to the left.
  What's under it shows as it goes: the card behind comes forward, whole wherever the front card has moved off it,
  or, with none behind, the tray fades back in.
  Let go past 40% of its width, or flick it right, and it flies off at the speed it was thrown, slowing as it goes;
  that dismisses it, like its x. Otherwise it springs back (a composition spring, carrying the release speed).
- **Clicks and swipes** on a card are queued in the TAP with the card's id; the daemon collects them. A click on the card
  or a button activates the app the way Windows would: its `INotificationActivationCallback` COM activator, a protocol
  URI, or launching a packaged app with the toast's arguments; a swipe or the x just dismisses it. An unpackaged app
  with no activator can only hear of a click through a handler in its own process, which nothing outside it can reach,
  so it's started as its Start menu entry would be.
- **Asking first.** `confirm_clicks` under `[toasts]`: `"card"` turns the card itself into the question, with what it
  would open and NO and YES; `"window"` asks in a window at the corner, with room for every path and argument; `"off"`
  doesn't ask. *SHUT UP, COMPUTER* makes it `"off"`. A card that's asking stays until it's answered.
- The card leaves at once; the next one's countdown starts when the daemon says (`toast front`).
- Do not disturb and full-screen apps still hold toasts back (`SHQueryUserNotificationState`).

## The watchdog

PimpMyShell never brings back an explorer that was simply ended: Task Manager and `taskkill` leave exit code 1, and
people do that on purpose. A **crash** is another matter. The daemon holds a handle to the explorer it hooked:

1. Explorer exits with an exception code (`0xC...`, at least `0x80000000`): a crash.
2. The daemon looks up the matching **Application Error, event 1000** record in the Application log (same app, same
   pid) and reads the faulting module.
   - `pms_tap*`, or XAML / composition modules the TAP drives (`Windows.UI.Xaml.dll`, `dcomp.dll`,
     `CoreMessaging.dll`, ...): bench at once.
   - Anything else: once is explorer's own business. Twice within 10 minutes: bench.
3. Hooking that keeps failing benches too: 3 failed injections in a row, or a TAP that had to be replaced 3 times in
   the same explorer (not counting swaps for a new build).
4. Windows normally restarts a crashed shell. If it hasn't within 12 seconds, the daemon starts explorer.exe, but only
   after a crash, never after it was ended.

**Benched** means: `reset` to the stock taskbar, native banners restored, the TAP told to `detach`, and no more
hooking for the rest of this run. You get a notification saying why, and the tray menu offers *Try again*,
which forgets the strikes and starts over.

## The updater

Nothing installs without you saying so.

- **When.** A minute into the first run, the daemon asks whether it may check by itself: *OK*, or closing the
  question, is yes; *No, don't check* is no. Unanswered, it checks nothing, and asks again next run. If yes, it checks
  a while after each start (1 to 30 minutes, at random) and then every 24 to 48 hours (2 to 6 after a failed check).
  The time of the last check is kept, so signing in again doesn't check again. *Check for updates* in the tray menu
  and the settings' *software update* checks right away.
- **Telling you.** A new version is downloaded and checked, then a notification says it's ready and the tray menu
  offers *Install PimpMyShell X…*, which asks first. A check from the settings ends the same way, with *Install X and restart*.
- **What's new.** The *stable* channel (`update_channel = "stable"`, the default) reads a tag from `update/stable.txt`
  on the repo's main branch. The *latest* channel takes the newest release, its tag read off where GitHub's
  `/releases/latest` redirects (a HEAD request). Only a version newer than the running one counts, except for the
  settings' *Reinstall stable* and *Reinstall latest release*, which fetch that version whatever this one is (still
  only as its signed manifest describes it, and still asking before installing).
- **Signed manifest.** Each release carries `pms-update.json` (version, installer name, size, SHA-256) and
  `pms-update.json.sig`, an Ed25519 signature over the manifest's exact bytes. The updater believes the manifest only
  if a public key compiled into `pms/src/signing.rs` verifies it (`verify_strict`), it names the very version asked
  for, and the installer is named `pms-setup-<version>.exe`. The tag itself is unsigned: at worst it can hold updates
  back.
- **Download.** Into `%TEMP%\pms-update`, size capped, hashed while it streams. Kept only if the size and SHA-256
  match the manifest. Hashed once more right before it runs, since it sat in `%TEMP%` meanwhile, and held open
  against writing from that hash until the installer has started.
- **Network.** HTTPS only, to GitHub's hosts only (`github.com`, `raw.githubusercontent.com`,
  `objects.githubusercontent.com`, `release-assets.githubusercontent.com`). Redirects are followed by hand and each one
  is checked the same way: no other host, no user, no port. TLS against Mozilla's roots, bundled. Timeouts and a size
  cap on every response.
- **Install.** The Inno Setup installer runs `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`. It stops the daemon, installs
  over it, and starts it again if it was running, and the settings too if they were open. Start at sign-in stays as you
  left it. The new daemon swaps its TAP in for the old one (see the reconcile
  loop above). The version goes into `tried.txt` first: if the install didn't take, the next check by itself says so
  (once a run) instead of looping; one you ask for downloads it again.
- Only an installed copy (running from `%LOCALAPPDATA%\Programs\PimpMyShell`) updates itself. A portable or dev build
  just says whether a newer release is out.
