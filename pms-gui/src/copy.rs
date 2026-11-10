//! What the settings say: each setting's title and description, the same in both windows (egui in main.rs, WinUI 3 in
//! winui.rs). Written as Microsoft's style guide has it: sentence case, "you", short and active, no period after a
//! fragment.
//!
//! A setting is known by its id, `table.key` as the config file has it (`meter.unit`, `hotmic.color`), or `key` for
//! a top-level one; a few say different things for different meter styles (`meter.names.disks`).

/// The title and description of setting `id`. An id with nothing written here shows itself.
pub fn setting(id: &str) -> (&str, &str) {
    match id {
        // General
        "theme" => ("Theme", "Choose dark or light for the taskbar's look, or follow your Windows mode"),
        "meter_height" => ("Meter height", "How much of the taskbar's height pies, graphs, and gauges use"),
        "bar_height" => ("Bar height", "How much of the taskbar's height meters made of bars use"),
        "interval_ms" => ("Update interval", "How often meters refresh, unless a meter has its own interval"),
        "software_gpus" => ("Include software graphics adapters", "Count Microsoft Basic Render Driver, which runs on the CPU, in GPU meters"),
        "rest_when_covered" => (
            "Rest while the taskbar's covered",
            "When a window fills the screen your taskbar's on (a game, a video, a presentation), nobody can see your TVs, so they stop drawing and the PIMPBOX rests. Twatch TVs with their sound off stop too; ones with sound go on playing it. All of it's back the moment the taskbar is",
        ),
        "rest_for_games" => (
            "Rest while you play",
            "When a game fills your screen and keeps your graphics card busy, your TVs show GAME ON and stop drawing, Twatch TVs make way, and the PIMPBOX stops, unless you've played it since it started. All come back about 20 seconds after the game",
        ),
        "max_fps" => ("Limit animation frame rate", "Cap ongoing animations, such as Hot Mic's and the notification rings. When it's off, they run at your display's refresh rate"),
        "battery_fps" => ("Frame rate on battery", "A lower cap while your PC runs on battery. 0 means no cap"),
        "value_font" => ("Meter font", "A fixed-width font keeps digits from shifting. Leave it empty to use Cascadia Mono, or Consolas"),
        "update_check" => (
            "Check for updates automatically",
            "We check GitHub a while after PimpMyShell starts (1 to 30 minutes), then every day or two. New versions download in the background, and you choose when to install them",
        ),
        "update_channel" => ("Update channel", "Stable gets a version once it has proven itself. Latest gets every release as soon as it's published"),

        "notify_explorer" => ("Tell me when Explorer could use a restart", "Windows can't unload what we've added to Explorer, so updates add up. When it's holding a lot, you get a notification. Explorer only restarts if you ask"),

        // TV and Twatch TV
        "shuffle_seconds" => ("Time between shuffles", "How long a shuffling TV stays on a channel. This is for all your TVs, and their remotes can change it too"),
        "shuffle_jitter" => ("Jitter", "Each shuffling TV changes channel a little late, by up to a quarter of the time above. When this is off, they all change together"),
        "console_video" => ("Video mode", "How big a picture the console puts out, which is what its window shows, as many times over as fits your screen. Nearly everything of its day used 480p. It changes at once, if the console's on"),
        "console_autostart" => ("Start the PIMPBOX when a TV tunes in", "Any TV landing on channel 6, even by shuffling, switches the console on. When this is off, channel 6 shows no signal until you switch it on"),
        "tv_intro" => ("Channel surf when a TV starts up", "When PimpMyShell starts, or you add a TV, it flips up through its channels before it settles. When this is off, it comes on at the channel it was left on"),
        "tv_intro_from" => ("Surf from", "The channel the run starts on"),
        "tv_intro_to" => ("Surf to", "Where the run ends, and the TV stays. On 6 with a game, the PIMPBOX starts"),
        "twatch.media_controls" => ("Show in Windows' media controls", "Where Windows shows what's playing: the volume flyout, the lock screen, and your keyboard's media keys. It's never by default, so a TwaTV doesn't take over your media keys"),
        "twatch.pictures" => ("Pictures instead of video", "A still of the channel every minute, for hardly any CPU or network. It doesn't need Streamlink. On battery you get these anyway"),
        "twatch.volume" => ("Volume", "How loud every TwaTV is, when its sound is on. Any set's remote can change it too"),
        "twatch.metered_pictures" => ("Pictures on a metered connection", "Video takes about 1 to 1.5 Mbps a set at 480p. While Windows says your connection is metered, like a phone hotspot, sets show a picture a minute instead"),
        "twatch.notify_live" => ("Tell me when a favorite goes live", "A notification, whichever TwaTV it ends up on. Select it to go to the channel"),
        "twatch.top_quality" => ("Highest quality", "It goes lower by itself while your PC is busy or the network can't keep up, and a small light on the set says so. The remote says why"),
        "twatch.more_sets" => ("Add TwaTVs by themselves", "While more favorites are live than you have TwaTVs, more appear, and they go away again when those streams end. Off, a purple plus by your TwaTVs offers one instead"),
        "twatch.higher_favorite" => ("When a higher favorite goes live", "What every set does, unless its remote is set to This stream or This channel. Whatever it does, its remote tells you"),
        "twatch.check_seconds" => ("How often to check each favorite", "Every favorite is looked at about this often, whatever its place in the list: in a random order, spread out, never two at once. Each look is one small request for a picture, with no account. On battery, it's twice as long"),
        "twatch.check_extra_seconds" => ("Plus a random wait of", "Added to each favorite's wait, a new amount every time, so that looks don't come like clockwork"),

        // Meters
        "meter.label" => ("Name", "Shown next to the meter and in its tooltip"),
        "meter.group" => ("Group", "Meters in different groups are separated by a divider"),
        "meter.sideways" => ("Name on its side", "Turned to read downwards, it takes up less room across. On by default for disk and network meters"),
        "meter.snug" => ("Attach to the previous meter", "No divider or space between this meter and the one before it"),
        "meter.caption" => ("Show name on the taskbar", "The name always appears in the tooltip"),
        "meter.style" => ("Style", "How the meter is drawn"),
        "meter.expr" => ("Expression", "A formula over counter names and built-in values, using + - * / ( ) min max abs. Pies and bars take one per slice or bar"),
        "meter.names" => ("Names", "One per slice or bar, shown in the tooltip"),
        "meter.names.disks" => ("Bar names", "What tooltips call each disk's two bars: write, then read"),
        "meter.names.nics" => ("Bar names", "What tooltips call each network adapter's two bars: upload, then download"),
        "meter.colors" => ("Colors", "One per slice or bar"),
        "meter.colors.cores" => ("Colors", "User time, then kernel time"),
        "meter.colors.disks" => ("Colors", "Write, then read"),
        "meter.colors.nics" => ("Colors", "Upload, then download"),
        "meter.color" => ("Color", "The meter's color"),
        "meter.max" => ("Full scale", "The value that fills the meter: a number or an expression. Choose None to scale automatically (a pie uses the sum of its slices)"),
        "meter.unit" => ("Unit", "%, bytes, bytes/s, or bits/s (sizes and rates scale to K, M, and G), or any other suffix"),
        "meter.decimals" => ("Decimal places", "Digits after the decimal point"),
        "meter.interval_ms" => ("Update interval", "How often this meter refreshes. By default, it uses the general update interval"),
        "meter.value" => ("Show value", "The value always appears in the tooltip"),
        "meter.tooltip" => ("Show tooltip", "Details appear when you point to the meter"),
        "meter.glow" => (
            "Glow",
            "Load glows with how busy each bar is now. Heat builds while a bar stays busy (full after about 8 seconds at 100%) and fades about 30 seconds after it stops",
        ),
        "meter.kernel" => ("Kernel time", "Where kernel time sits in each thread's bar: below user time, or above it"),
        "meter.bar_width" => ("Bar width", "Each bar's width before bars narrow to fit. When it's off, bars fit automatically"),
        "meter.vendors" => ("Label by manufacturer", "Label each GPU by its manufacturer, and color its bars to match"),
        "meter.vendor_label" => ("Manufacturer labels", "Short labels (NV, RAD) or stock tickers (NVDA, AMD)"),
        "meter.hold_seconds" => ("Hide idle GPUs after", "The GPU that was busy most recently stays visible. 0 keeps every GPU visible"),
        "meter.hold_seconds.disks" => ("Hide idle disks after", "The disk Windows is on stays visible. 0 keeps every disk visible"),
        "meter.hold_seconds.nics" => ("Hide idle adapters after", "The adapter your internet connection uses stays visible. 0 keeps every adapter visible"),
        "meter.engines_fall_off" => ("Hide idle engines", "Hide individual idle engines too, not just whole GPUs"),
        "meter.per_gpu" => (
            "One pie per GPU",
            "A pie for each graphics adapter with at least 1 GB of dedicated memory, in its manufacturer's color. We fill in the expression, full scale, and color",
        ),
        "meter.label_inside" => ("Name inside the pie", "Put the name inside the pie: never, only when space is short, or always"),
        "meter.slice_glow" => ("Glowing slice", "The name of a slice that glows, as it's written under Names. Leave it empty for none"),
        "meter.slice_glow_extend" => ("Slice stands out", "How far the slice can stick out past the pie while its amount is changing. At rest it doesn't"),
        "meter.slice_glow_radius" => ("Glow radius", "How far the glow reaches"),
        "meter.slice_glow_strength" => ("Glow strength", "How bright the glow is"),
        "meter.slice_glow_color" => ("Glow color", "Empty uses the slice's own color"),
        "meter.slice_glow_pulse_ms" => ("Glow pulse", "How long a slow pulse takes. 0 holds it steady"),
        "meter.pressure" => ("Pressure point", "How full the first slice gets before the glow around the pie grows stronger"),
        "meter.time_axis" => ("Time scale", "Logarithmic gives recent values the most room. Linear spaces every value evenly"),
        "meter.history_seconds" => ("History length", "How far back the graph goes"),
        "meter.show_above" => ("Show only above", "Hide the meter until its value reaches this number or expression"),
        "meter.show_when" => (
            "Show only when",
            "Hide the meter until another value reaches a threshold, and hide it again once that value drops back. For example, show the pagefile only when memory runs low",
        ),

        // Counters
        "counter.name" => ("Name", "What expressions call this counter"),
        "counter.path" => ("Path", "\\Object(instance)\\Counter, with * for every instance"),
        "counter.agg" => ("Combine", "How instances (*) combine. GPU engine matches Task Manager's GPU %"),
        "counter.default" => ("Fallback", "A value to use when the counter can't be read"),

        // Taskbar look
        "look.tint" => ("Tint", "Laid over the blurred desktop. Its transparency sets how much shows through"),
        "look.grain" => ("Grain", "Noise over the tint"),
        "look.gaps" => ("Fill between buttons", "Color the empty space between taskbar buttons"),
        "start_glow.color" => ("Color", "The glow's color"),
        "start_glow.hover" => ("Brightness on hover", "When you point to the Start button"),
        "start_glow.press" => ("Brightness when pressed", "While you press the Start button"),
        "start_glow.open" => ("Brightness while Start is open", "Also when you open Start with the Windows key"),
        "start_glow.hover_ms" => ("Fade time", "How long the glow takes to fade in and out"),
        "start_glow.press_ms" => ("Fade time when pressed", "Usually quicker"),
        "start_glow.hover_radius" => ("Size on hover", "How far the glow reaches, in pixels"),
        "start_glow.press_radius" => ("Size when pressed", "How far the glow reaches, in pixels"),
        "app_glow.hover" => ("Brightness on hover", "When you point to an app button"),
        "app_glow.press" => ("Brightness when pressed", "While you press an app button"),
        "app_glow.hover_ms" => ("Fade time", "How long the glow takes to fade in and out"),
        "app_glow.press_ms" => ("Fade time when pressed", "Usually quicker"),
        "app_glow.hover_radius" => ("Size on hover", "How far the glow reaches, in pixels"),
        "app_glow.press_radius" => ("Size when pressed", "How far the glow reaches, in pixels"),

        // Hot Mic
        "hotmic.color" => ("Alert color", "The color of the alert"),
        "hotmic.period_ms" => ("Pulse length", "One pulse of the alert. 0 keeps it steady"),
        "hotmic.low" => ("Lowest opacity", "How faint the pulse gets"),
        "hotmic.high" => ("Highest opacity", "How strong the pulse gets"),
        "hotmic.broad_ms" => ("Full taskbar alert", "How long the whole taskbar alerts before the alert settles around its label"),
        "hotmic.ignore" => ("Excluded apps", "Apps that never trigger the alert. Enter part of an app's name or path"),
        "hotmic.position" => ("Where its label goes", "The label naming the apps: first, before your meters and TVs, or last, beside the tray. The Widgets page has it in its place too"),

        // Notifications
        "toasts.duration_ms" => (
            "Display time",
            "How long a notification stays in front, or at most 4 seconds if more are waiting. Those that ask for longer get at least 25 seconds. Alarms, calls, and reminders stay until you respond",
        ),
        "toasts.confirm_clicks" => ("Ask before opening", "When you select a notification, show what it's about to open and let you say no. A window has room for every detail"),
        "toasts.position" => ("Where they appear", "Where cards stop. Left of the clock, they cover the tray. Left of your first TV, or of the Hot Mic label, that and everything to its right stays in sight; when it isn't showing, cards stop at the tray. On a full taskbar they cover the whole tray"),
        "toasts.style" => ("Card style", "Standard matches Windows. Pimped turns cards gold, with a sheen that sweeps across them"),
        "toasts.ignore" => ("Excluded apps", "Apps that keep their standard pop-ups. Enter part of an app's name or ID"),

        _ => (id, ""),
    }
}

/// The key the config file has for setting `id`: its last part (`meter.names.disks` is `names`).
pub fn key(id: &str) -> &str {
    let mut parts = id.split('.');
    let first = parts.next().unwrap_or(id);
    parts.next().unwrap_or(first)
}

/// Why the classic window is showing, where nobody asked for it.
pub const CLASSIC_NO_RUNTIME: &str = "You're seeing the classic settings window because this PC doesn't have the Windows App Runtime 2.4 or later, which the new window needs. It's a Microsoft component that PimpMyShell doesn't install for you. Every setting works the same here.";
pub const CLASSIC_FAILED: &str = "You're seeing the classic settings window because the new window couldn't start on this PC. Every setting works the same here.";
pub const RUNTIME_DOWNLOAD: (&str, &str) = ("Get the Windows App Runtime", "https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads");

/// The first run's page on each of the extras.
pub const WELCOME_TV: (&str, &str) = ("Your taskbar got cable", "There's a little TV down there now. Go on, point at it.");
pub const WELCOME_TV_POINTS: [(&str, &str, &str); 3] = [
    ("\u{E8B2}", "The remote's in the tooltip", "Channels, shuffle, power, and another TV if one isn't enough. They can all watch something different"),
    ("\u{E7F4}", "Six channels of nonsense", "A test card, a crab that never hits the corner, static, the PimpCast, a console, and a very angry man from 1986"),
    ("\u{E7E8}", "Off means out of your way", "Switch one off and a few seconds later it's just a small button. Click it to bring the set back"),
];
pub const WELCOME_TV_ON: (&str, &str) = ("Keep a TV on my taskbar", "You can add more, or take them all away, in Settings, under TV. Its games are under PIMPBOX");
/// What's new, for somebody who had PimpMyShell before it had these.
pub const WHATS_NEW: (&str, &str) = ("Your taskbar got toys", "Since you last looked: a TV with six channels, a console on channel 6, and Twitch on a little flat-screen. Three quick stops, and each is yours to keep or skip.");
pub const WELCOME_PIMPBOX: (&str, &str) = ("Pimps on Mars.", "Try it on Channel 6. Play games like Doom, Freedoom, etc. Click on the TV to pop a window on your desktop. Alt+Tab when the boss walks by (Alt+F4 works too. Game state isn't nuked.)");
pub const WELCOME_TWATCH: (&str, &str) = ("Twatch TV", "Twitch, on a little flat-screen, live. Name your favorite channels and the best one that's on just plays. No account, no sign-in, nothing to click.");
pub const WELCOME_TWATCH_POINTS: [(&str, &str, &str); 3] = [
    ("\u{E734}", "Best first", "Your favorites go in order. When a better one goes live, it takes over. Switch on more sets, and up to three come out by themselves when a few are on"),
    ("\u{E945}", "Easy on your PC", "480p by default, and lower by itself when your PC or the network is struggling"),
    ("\u{E83F}", "Easy on your battery", "Off the charger it shows a new picture every minute instead, and video comes back when you plug in"),
];
pub const WELCOME_TWATCH_ON: (&str, &str) = ("Put a Twatch TV on my taskbar", "Then add your channels and get Streamlink in Settings, under Twatch TV");
/// The same, when there are no channels yet: Settings opens there at the end.
pub const WELCOME_TWATCH_NEXT: &str = "When you're done here, Settings opens on Twatch TV, to add your channels and get Streamlink for video";
/// And while it's switched off.
pub const WELCOME_TWATCH_OFF: &str = "Switch it on, and Settings opens on Twatch TV when you're done, to add your channels and get Streamlink for video";

/// Lines of the settings file that aren't settings, which are left out so that the rest loads.
pub const IGNORED_LINES: &str = "Everything else loaded. A setting that couldn't be used is back to its default, a line that isn't a setting is left out, and each is marked in red where it belongs. Saving a change here leaves them out of the file. To keep one, fix it in the file and save it.";

/// Why the favorites and the network settings have files of their own.
pub const PRIVATE_FILES: &str = "Your favorites and network settings are kept out of the settings file, so you can share that file for help without them.";

/// What's in the logs, and what isn't (logfile.rs, the daemon's lines on toasts, system.rs's report): its title, what's left out,
/// and what each file can say about you. Kept to what the code logs: a new kind of line that says something about somebody goes here.
pub const LOG_PRIVACY: (&str, &str) = (
    "What your logs can say about you",
    "Your Windows user name, what your notifications say, your proxy and your list of favorites are left out. The rest is there to help fix things, and people often share logs: read one before you do.",
);
pub const LOG_CONTENTS: &[(&str, &[&str])] = &[
    (
        "daemon.log",
        &[
            "When you used your PC: every line has the date and time, in UTC, so not where you are",
            "The names of apps that notify you, and of apps that use your microphone",
            "Twitch channels from your favorites, when something about one comes up",
            "Which game the PIMPBOX played",
            "When a game was in front of everything, though not which one",
            "File and folder paths, with your user name taken out",
            "Your version of Windows, and other software that's loaded into Explorer or changes the taskbar",
            "Error messages from Windows, and from Streamlink",
            "Whether you use a proxy, though never its address",
        ],
    ),
    ("tap.log", &["When you used your PC: every line has the date and time, in UTC", "Your TVs' names, and errors from inside the taskbar"]),
    (
        "A saved report, besides the end of both logs",
        &["Your settings file, with your meters and the apps you've excluded from Hot Mic and Notification Toast", "How your taskbar is laid out: what's in it, without any window's title"],
    ),
];

/// LOG_CONTENTS, as bulleted lines.
pub fn log_contents() -> String {
    LOG_CONTENTS.iter().map(|(file, lines)| format!("{file}\n{}", lines.iter().map(|l| format!("  •  {l}")).collect::<Vec<_>>().join("\n"))).collect::<Vec<_>>().join("\n\n")
}

/// The small print for what floats behind the settings window (dollars.rs).
pub const LEGAL: &str = include_str!("legal.txt");

/// Choices: each value as the file has it, and what the window calls it.
pub const THEMES: &[(&str, &str)] = &[("auto", "Use my Windows mode"), ("dark", "Dark"), ("light", "Light")];
pub const HEIGHTS: &[(&str, &str)] = &[("compact", "Compact (75%)"), ("tall", "Tall (88%)"), ("full", "Full")];
pub const CHANNELS: &[(&str, &str)] = &[("stable", "Stable"), ("latest", "Latest")];
pub const CLEAR_SHADERS: (&str, &str) = ("Compiled shaders", "The PIMPBOX compiles its boot animation for your graphics card once, and keeps the result. Clearing it makes it compile again the next time it comes on, which takes a moment");
pub const CLEAR_SHADERS_ASK: &str = "This switches the PIMPBOX off, which ends the game it's playing: anything you haven't saved is lost. Its compiled shaders are then deleted. The next time it comes on it compiles them again, showing a small wheel while it does.";
pub const CONSOLE_VIDEO: &[(&str, &str)] = &[("480p", "480p (640 by 480)"), ("720p", "720p (1280 by 720)"), ("1080i", "1080i (1920 by 1080)")];
pub const TOAST_STYLES: &[(&str, &str)] = &[("standard", "Standard"), ("pimped", "Pimped")];
pub const TOAST_POSITIONS: &[(&str, &str)] = &[("clock", "Left of the clock"), ("tray", "Left of the system tray"), ("tv", "Left of your first TV"), ("mic", "Left of the Hot Mic label")];
pub const CONFIRMS: &[(&str, &str)] = &[("card", "In the notification"), ("window", "In a window"), ("off", "Don't ask")];
pub const MIC_POSITIONS: &[(&str, &str)] = &[("tray", "Last, beside the tray"), ("start", "First, before your meters and TVs")];
pub const STYLES: &[(&str, &str)] = &[
    ("history", "History graph"),
    ("bar", "Bar"),
    ("pie", "Pie"),
    ("bars", "Bars"),
    ("cores", "CPU threads"),
    ("engines", "GPU engines"),
    ("disks", "Disks"),
    ("nics", "Network adapters"),
];
pub const UNITS: &[(&str, &str)] = &[("", "None"), ("%", "%"), ("bytes", "Bytes"), ("bytes/s", "Bytes/s"), ("bits/s", "Bits/s")];
pub const GLOWS: &[(&str, &str)] = &[("level", "Load"), ("heat", "Heat"), ("off", "Off")];
pub const KERNELS: &[(&str, &str)] = &[("bottom", "Below user time"), ("top", "Above user time")];
pub const VENDOR_LABELS: &[(&str, &str)] = &[("short", "Short (NV, RAD)"), ("ticker", "Tickers (NVDA, AMD)")];
pub const INSIDE: &[(&str, &str)] = &[("never", "Never"), ("auto", "When space is short"), ("always", "Always")];
pub const TIME_AXES: &[(&str, &str)] = &[("log", "Logarithmic"), ("linear", "Linear")];
pub const AGGS: &[(&str, &str)] = &[("sum", "Sum"), ("max", "Maximum"), ("min", "Minimum"), ("avg", "Average"), ("engine", "GPU engine")];

/// Pages: title, and what's on them.
pub const GENERAL: (&str, &str) = ("General", "");
pub const METERS: (&str, &str) = ("Meters", "Your meters, in their order on the taskbar, left to right. Select one to change it, and move it with the arrows. Where they go among your TVs is on the Widgets page.");
/// When PimpMyShell gets out of the way, and what it does by itself.
pub const POWER: (&str, &str) = ("Power & performance", "When PimpMyShell gets out of the way: when nobody can see it, for your games, on battery, and for your memory and your data.");
pub const POWER_BY_ITSELF: &str = "Some of it happens by itself. On battery, a TV you leave alone sleeps, and Twatch TVs show pictures. When memory or video memory runs short, Twatch TVs make way, and so does the PIMPBOX, unless you've been playing it. When the PC is busy, TVs draw at half speed.";
/// Under Twatch TV's settings: where its pictures went.
pub const TWATCH_POWER: &str = "Pictures instead of video, to save memory or data, are under Power & performance.";
/// The logs' page.
pub const LOGS: (&str, &str) = ("Logs", "What PimpMyShell writes down as it goes, to help fix things when they go wrong.");
/// The page of what's along the taskbar, and its notes.
pub const WIDGETS: (&str, &str) = ("Widgets", "What's along the taskbar beside the system tray, in its order: your meters, together; each TV and Twatch TV; and the Hot Mic label, at one end or the other. When space runs short they squeeze up, then the ones nearest the tray hide first (never the Hot Mic label).");
pub const WIDGET_MIC_FIRST: &str = "Its label, while an app uses your mic: first, before your meters and TVs";
pub const WIDGET_MIC_LAST: &str = "Its label, while an app uses your mic: last, beside the tray";
pub const WIDGETS_NONE: &str = "Nothing's on the taskbar yet: add meters, a TV or a Twatch TV on their pages.";
pub const WIDGETS_NO_MIC: &str = "Hot Mic is off. Switched on, under Hot Mic, its label goes at one end of these.";
/// A program of PimpMyShell's (`file`) isn't where the rest are.
pub fn part_missing(file: &str) -> String {
    let dir = pms::parts::path(file).and_then(|p| p.parent().map(pms::shown)).unwrap_or_default();
    format!(
        "It should be with the rest of PimpMyShell, in {dir}, but it isn't, so {}. An antivirus program or a cleanup tool may have taken it away. Installing PimpMyShell again puts it back, and keeps your settings.",
        pms::parts::without(file)
    )
}
/// Under the list of what's in the taskbar.
pub const ORDER_NOTE: &str = "Twatch TVs that come and go by themselves follow the last one, and so do their buttons. Notification cards slide in over the tray's end: where they stop is under Notification Toast.";
pub const COUNTERS: (&str, &str) = (
    "Performance counters",
    "The Windows performance counters your meters read, by their English paths. Use their names in meter expressions, along with ram_total, vram_total, swap_total, swap_used, and cpu_count. To find paths, run typeperf -q.",
);
pub const LOOK: (&str, &str) = ("Taskbar look", "Replace the taskbar's standard acrylic with your own background, with a look for dark mode and one for light mode.");
pub const TV: (&str, &str) = ("TV", "A little TV in your taskbar. Point at it for its remote: channels, shuffle, power, and another TV if one isn't enough. Channel 6 is the PIMPBOX, your console: its games are under PIMPBOX.");
pub const TV_HELP: &str = "On the TV: click the picture to stop shuffling. On channel 6, click again to play. On battery, a TV you leave alone sleeps.";
pub const PIMPBOX: (&str, &str) = ("PIMPBOX", "A console on channel 6 of your TVs. Give it a game, and any TV tuned to 6 plays it. It plays Doom's games with POOM (room).");
pub const PIMPBOX_HELP: &str = "On a TV on channel 6, click the picture to play: a window opens on your desktop. Esc opens the game's menu. Alt+Tab, or clicking away, lets go of the mouse. Closing the window pauses the game. Changing the channel doesn't turn the console off: it has its own power button on the TV's remote. Asking for channel 6 with no game brings you here.";
/// Where the order of what's in the taskbar is changed, for the pages of what's in it.
pub const ORDER_HINT: &str = "To move it along the taskbar, use the arrows on the Widgets page.";
pub const TWATCH: (&str, &str) = ("Twatch TV", "A flat-screen set that plays your favorite Twitch channels while they're live, as live video in your taskbar. Point at it for its remote, and click the picture to open the channel. On battery it shows pictures instead, and goes back to video when you plug in. Live video needs Streamlink, just below.");
pub const TWATCH_FAVORITES: &str = "Your favorite channels, best first. Nothing else about you or your account is used: there's no sign-in.";
pub const TWATCH_AS_TEXT: &str = "One channel per line, best first. Names, @names and twitch.tv links all work. Nothing changes until you save.";
pub const TWATCH_HIGHER: [(&str, &str); 2] = [("switch", "Switch to it"), ("stay", "Stay on what's playing")];
pub const TWATCH_CHECKS: [(&str, &str); 6] = [("60", "Every minute"), ("120", "Every 2 minutes"), ("300", "Every 5 minutes"), ("600", "Every 10 minutes"), ("900", "Every 15 minutes"), ("1800", "Every 30 minutes")];
pub const TWATCH_CHECK_EXTRAS: [(&str, &str); 5] = [("0", "None"), ("60", "Up to a minute"), ("120", "Up to 2 minutes"), ("300", "Up to 5 minutes"), ("600", "Up to 10 minutes")];
pub const TWATCH_QUALITIES: [(&str, &str); 5] = [("1080p60", "1080p, 60 frames a second"), ("720p60", "720p, 60 frames a second"), ("720p", "720p, 30 frames a second"), ("480p", "480p, 30 frames a second"), ("360p", "360p")];
pub const TWATCH_MORE_SETS: [(&str, &str); 3] = [("0", "Off"), ("1", "Up to one more"), ("2", "Up to two more")];
/// The TV's channels, as the settings name them.
pub const TV_CHANNELS: [(&str, &str); 6] = [("1295", "1295: Captain Midnight"), ("2", "2: Test card"), ("3", "3: The crab"), ("4", "4: Static"), ("5", "5: PimpCast"), ("6", "6: PIMPBOX")];
pub const TWATCH_MEDIA_CONTROLS: [(&str, &str); 3] = [("never", "Never"), ("unmuted", "While its sound is on"), ("playing", "Whenever it's playing video")];
/// Above the Twatch TV page while there are favorites, video is wanted, and Streamlink isn't here.
pub const TWATCH_NO_STREAMLINK: (&str, &str) = ("Live video needs Streamlink", "You have favorites, but without Streamlink your TwaTVs can't play them. Download it below, or choose Always under Pictures instead of video, further down, for a still every minute.");
pub const TWATCH_PICTURES: [(&str, &str); 4] = [("never", "Never"), ("initial", "Until you click a TwaTV"), ("low_memory", "When RAM or VRAM is low"), ("always", "Always")];
pub const RESET_ALL: (&str, &str) = ("Reset settings", "Everything back to how it came. Your Twatch TV favorites, network settings, games and answer about updates stay");
pub const RESET_ALL_ASK: &str = "Every setting goes back to how it came, Twatch TV's included. Your Twatch TV favorites, your network settings, your games and your answer about updates stay. What your settings are now is kept beside them, as pms.toml.before-reset.";
pub const TWATCH_NETWORK: &str = "What Twatch TV asks of the internet, and how to send it through a proxy, is under Network.";
pub const TWATCH_NOTICE: &str = "Twitch is a trademark of Twitch Interactive, Inc. PimpMyShell isn't affiliated with Twitch. Streamlink is a separate program under the BSD 2-Clause License, built by GitHub's runners from its own source.";
pub const TV_NOTICE: &str = "DOOM is a registered trademark of id Software LLC, a ZeniMax Media company. PimpMyShell isn't affiliated with or endorsed by id Software, ZeniMax, Bethesda or Microsoft. The PIMPBOX, pms-pimpbox.exe, plays them with POOM (room), a Doom engine, and is under the GNU GPL version 2, with no warranty; its source is in PimpMyShell's repository on GitHub. The rest of PimpMyShell is under the MIT License, and each game keeps its own license.";
pub const NETWORK: (&str, &str) = ("Network", "Everything PimpMyShell uses the internet for, and a proxy to send any of it through. There's no account, no analytics and no ads: nothing about you or your PC is sent anywhere.");
pub const PROXY: (&str, &str) = (
    "SOCKS5 proxy",
    "socks5h:// looks names up through the proxy too; socks5:// looks them up on this PC. Add user:password@ before the host if it needs them. Leave it empty for none",
);
pub const PROXY_ISOLATE: (&str, &str) = (
    "A random user name and password for each",
    "For Tor's stream isolation: each of the uses below goes with a user name and password of its own, made up the first time it's needed and kept until PimpMyShell exits, so Tor puts them on circuits of their own. They take the place of any in the address",
);
/// What a proxy here can't promise, and what can.
pub const PROXY_LIMITS: &str = "We test all of this as we build it, but nothing is airtight, least of all the software PimpMyShell gets from elsewhere for you, like Streamlink: it does its own networking, and changes with its own releases.\n\nIf it matters that nothing gets out except through your proxy, have Windows make sure. Windows Firewall, or another filter built on the Windows Filtering Platform, can keep these from going anywhere but your proxy:\n\npmsd.exe, pms-gui.exe, pms-twatch.exe and streamlink.exe";
/// The network settings' file won't load.
pub const NETWORK_UNREAD: &str = "PimpMyShell can't tell whether you wanted a proxy, so nothing goes online until pms-network.toml is fixed: no checking who's live, no Twatch TV video, no update checks and no downloads. Fix the file and save it, or apply a proxy here to replace it. To go without a proxy, delete the file.";

/// A proxy is set, and nothing goes through it.
pub const PROXY_UNUSED: (&str, &str) = ("Nothing goes through your proxy", "Everything still goes straight to where it's going. Switch on what should use the proxy below, or select Turn on all.");
pub const PROXY_STRICT: &str = "What's switched on here never goes without the proxy: if the proxy can't be reached, it fails until it can. What's switched off goes straight to where it's going.";
pub const NETWORK_ELSE: &str = "That's all of it. Windows does what it does by itself, and the notifications PimpMyShell shows come from your apps, not from the internet. Streamlink and Twatch TV's player, pms-twatch.exe, are the only other programs PimpMyShell starts that use the network, and both are covered above.";
pub const HOTMIC: (&str, &str) = (
    "Hot Mic",
    "Know when an app is using your microphone. The whole taskbar alerts first, then the alert settles around a label naming the apps, beside the system tray, or before your meters and TVs. An app that stops stays listed for 10 seconds.",
);
pub const TOASTS: (&str, &str) = ("Notification Toast", "Show notifications in the taskbar instead of as pop-ups. Their buttons still work, and they stay in notification center.");

#[cfg(test)]
mod tests {
    use super::*;

    /// UTF-8 read as Windows-1252 and saved again turns a middle dot, a bullet or a dash into pairs of odd letters. It still
    /// builds and runs, so nothing else would notice. (The pairs are spelled out by code point, or this file would trip.)
    #[test]
    fn no_garbled_text() {
        let garbled = [['\u{c2}', '\u{b7}'], ['\u{e2}', '\u{2014}'], ['\u{e2}', '\u{20ac}'], ['\u{c3}', '\u{a9}']].map(|p| p.iter().collect::<String>());
        let sources = [
            ("main.rs", include_str!("main.rs")),
            ("winui.rs", include_str!("winui.rs")),
            ("model.rs", include_str!("model.rs")),
            ("copy.rs", include_str!("copy.rs")),
            ("dollars.rs", include_str!("dollars.rs")),
            ("tooltip.rs", include_str!("../../pms-tap/src/tooltip.rs")),
            ("meters.rs", include_str!("../../pms-tap/src/meters.rs")),
            ("toast.rs", include_str!("../../pms-tap/src/toast.rs")),
            ("daemon meters.rs", include_str!("../../pms/src/meters.rs")),
            ("notices.rs", include_str!("../../pms/src/notices.rs")),
            ("tray.rs", include_str!("../../pms/src/tray.rs")),
        ];
        for (name, text) in sources {
            for g in &garbled {
                assert!(!text.contains(g.as_str()), "{name} has garbled text: {g}");
            }
        }
    }

    #[test]
    fn keys_and_style() {
        assert_eq!(key("interval_ms"), "interval_ms");
        assert_eq!(key("meter.names.disks"), "names");
        assert_eq!(key("start_glow.hover_ms"), "hover_ms");
        // Titles are sentence case without a period; descriptions don't end in one either (they're fragments, or
        // end with a fragment).
        for id in ["theme", "update_check", "meter.show_when", "hotmic.broad_ms", "toasts.ignore", "look.tint"] {
            let (title, desc) = setting(id);
            assert!(!title.ends_with('.') && !desc.ends_with('.'), "{id}");
            assert!(title.chars().next().is_some_and(char::is_uppercase), "{id}");
        }
    }
}
