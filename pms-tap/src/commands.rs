//! Command interpreter. Runs on the pipe thread; anything touching XAML hops to the UI thread.

use std::fmt::Write as _;
use std::mem::transmute_copy;

use windows_core::{BSTR, HSTRING, PCWSTR};

use crate::xamlom::*;
use crate::{Ctx, TREE, dfs_order, lock, props, render_tree, run_on_ui};

const HELP: &str = r#"commands:
  ping                              is anybody home
  tree [elem]                       mirrored visual tree (optionally just a subtree)
  find <elem>                       list elements matching a selector
  props <target> [filter]           effective property values (non-default unless filtered)
  set <target> <Prop> <value> [Type]
                                    create a value from a string and assign it; Type defaults to
                                    the property's current value type. value `ref:0x..` assigns
                                    an existing object handle instead.
  clear <target> <Prop>             reset to the styled/default value
  new <Type> [value]                create an object, prints its handle
  add <parent> <child> [index]      insert child (handle or selector) into a collection; a panel
                                    means its Children. index defaults to the end.
  remove <parent> <index>           remove the collection's (or panel's) child at index
  detach                            unsubscribe and go inert (`pms inject` does this for you)
  status                            `tap=<copy id> taskbar=<BackgroundFill handle|none> backdrop=..
                                    alert=.. meters=N toast=..`; the daemon re-applies when tap/taskbar change
  reset                             remove everything PimpMyShell added (backdrop, alert, meters)

  alert <#AARRGGBB> [ms] [low] [high] | alert off
                                    wash the whole taskbar (below meters and buttons) in a color
                                    breathing between low and high opacity; ms 0 = steady

  toast show ms=<n> [title= body= app= attribution= icon= logo= circle=1 image= b=<label>..]
                                    slide the tray away and show a notification card in its
                                    place for ms (the countdown bar). Normally driven by the daemon.
  toast hide | status
  toast events                      clicks since last asked: open | action <i> | close

  backdrop on|off|status            swap the stock acrylic for PimpMyShell's composition backdrop
  backdrop tint <#AARRGGBB>         tint over the blurred backdrop (alpha = opacity)
  backdrop grain <0..1>             grain strength; 0 is off
  backdrop layer <#AARRGGBB>        solid color on our layer (between tint and grain)
  backdrop pulse <#AARRGGBB> [ms] [low] | pulse off
                                    breathe the layer's opacity between low and 1
  backdrop gaps <#AARRGGBB> | gaps off
                                    paint the free space between buttons; follows layout live

  meters add <label> <#color> <max|auto>   add a meter cell (sparkline + value)
  meters push <v0> <text0> [<v1> <text1> ..] push one sample per meter, in order
  meters clear | status             (normally driven by `pms meters <config.toml>`)

  layout                            button bounds + free intervals (DIPs, taskbar-relative);
                                    starts live tracking. `layout stop` ends it.
                                    (`pms layout watch` re-prints on every change)

selectors:
  elem    = Name | Type#Name | Type | 0xHANDLE, optional `:N` for the Nth match (tree order).
            Type may be a suffix: Rectangle#BackgroundFill, TaskListButton:2
  target  = elem[/Prop[/Prop..]]   follows object-valued properties, e.g. BackgroundFill/Fill
"#;

pub(crate) type R<T> = Result<T, String>;

pub fn execute(line: &str) -> String {
    let args = crate::ipc::tokenize(line);
    let result = std::panic::catch_unwind(|| dispatch(&args)).unwrap_or_else(|_| Err("command panicked".into()));
    match result {
        Ok(s) => s,
        Err(e) => format!("error: {e}"),
    }
}

fn arg<'a>(args: &'a [String], i: usize, what: &str) -> R<&'a str> {
    args.get(i).map(String::as_str).ok_or_else(|| format!("missing <{what}>"))
}

fn dispatch(args: &[String]) -> R<String> {
    let Some(cmd) = args.first() else { return Ok(String::new()) };
    let rest = &args[1..];
    let owned = |i: usize, what: &str| arg(rest, i, what).map(str::to_string);
    match cmd.as_str() {
        "help" | "?" => Ok(HELP.into()),
        "ping" => Ok(format!("pong from explorer pid {}", std::process::id())),
        "detach" => crate::detach(),
        "status" => status(),
        // pgo-dump: an instrumented build (cargo xtask pgo) writes what it recorded so far.
        #[cfg(pms_pgo)]
        "pgo-dump" => {
            unsafe extern "C" {
                fn __llvm_profile_write_file() -> i32;
            }
            Ok(format!("pgo-dump: {}", unsafe { __llvm_profile_write_file() }))
        }
        // buttonglow start ..| off: the Start button's glow (glow.rs).
        "buttonglow" => {
            let rest = rest.to_vec();
            ui(move |ctx| crate::glow::command(ctx, &rest))
        }
        // pace <fps>: the long-running animations' frame rate (pace.rs).
        "pace" => crate::pace::command(rest),
        // theme light|dark: the shell's mode; our ink follows it.
        "theme" => {
            let light = rest.first().map(String::as_str) == Some("light");
            crate::LIGHT.store(light, std::sync::atomic::Ordering::Relaxed);
            ui(crate::meters::retheme)
        }
        // batch "<command>" "<command>" ..: one UI-thread job (one layout pass) for all of them.
        "batch" => {
            let subs = rest.to_vec();
            run_on_ui(move |_| subs.iter().map(|s| execute(s)).filter(|r| !r.is_empty()).collect::<Vec<_>>().join("
"))
        }
        "reset" => run_on_ui(crate::reset).map(|s| format!("reset; {s}")),
        "alert" => {
            let rest = rest.to_vec();
            ui(move |ctx| crate::alert::command(ctx, &rest))
        }
        "toast" if rest.first().map(String::as_str) == Some("events") => Ok(crate::toast::drain_events()),
        "toast" => {
            let rest = rest.to_vec();
            ui(move |ctx| crate::toast::command(ctx, &rest))
        }
        "dwm" => crate::dwm::command(rest),
        "meters" => {
            let rest = rest.to_vec();
            ui(move |ctx| crate::meters::command(ctx, &rest))
        }
        "layout" => {
            let rest = rest.to_vec();
            ui(move |ctx| crate::layout::command(ctx, &rest))
        }
        "backdrop" => {
            let rest = rest.to_vec();
            ui(move |ctx| crate::backdrop::command(ctx, &rest))
        }
        "tree" => {
            let root = rest.first().map(|s| resolve_elem(s)).transpose()?;
            Ok(render_tree(root))
        }
        "find" => find(arg(rest, 0, "elem")?),
        "props" => {
            let target = owned(0, "target")?;
            let filter = rest.get(1).cloned();
            ui(move |ctx| {
                let h = resolve_target(ctx, &target)?;
                let body = props::dump(&ctx.service, h, filter.as_deref()).map_err(|e| e.to_string())?;
                Ok(format!("{target} [{h:#x}]\n{body}"))
            })
        }
        "set" => {
            let (target, prop, value) = (owned(0, "target")?, owned(1, "Prop")?, owned(2, "value")?);
            let ty = rest.get(3).cloned();
            ui(move |ctx| {
                let h = resolve_target(ctx, &target)?;
                let index = prop_index(ctx, h, &prop)?;
                let value_handle = match value.strip_prefix("ref:") {
                    Some(r) => parse_handle(r)?,
                    None => {
                        let ty = match ty {
                            Some(t) => t,
                            None => infer_type(ctx, h, &prop)?,
                        };
                        create_instance(ctx, &ty, &value)?
                    }
                };
                hr(unsafe { ctx.service.SetProperty(h, value_handle, index) }, "SetProperty")?;
                Ok(format!("{target}.{prop} = {value}"))
            })
        }
        "clear" => {
            let (target, prop) = (owned(0, "target")?, owned(1, "Prop")?);
            ui(move |ctx| {
                let h = resolve_target(ctx, &target)?;
                let index = prop_index(ctx, h, &prop)?;
                hr(unsafe { ctx.service.ClearProperty(h, index) }, "ClearProperty")?;
                Ok(format!("{target}.{prop} cleared"))
            })
        }
        "new" => {
            let ty = owned(0, "Type")?;
            let value = rest.get(1).cloned().unwrap_or_default();
            ui(move |ctx| Ok(format!("{:#x}", create_instance(ctx, &ty, &value)?)))
        }
        "add" => {
            let (parent, child) = (owned(0, "parent")?, owned(1, "child")?);
            let index = rest.get(2).map(|i| i.parse::<u32>().map_err(|e| e.to_string())).transpose()?;
            ui(move |ctx| {
                let p = children_of(ctx, resolve_target(ctx, &parent)?)?;
                let c = resolve_target(ctx, &child)?;
                let index = match index {
                    Some(i) => i,
                    None => collection_count(ctx, p)?,
                };
                hr(unsafe { ctx.service.AddChild(p, c, index) }, "AddChild")?;
                Ok(format!("added {c:#x} to {p:#x} at {index}"))
            })
        }
        "remove" => {
            let parent = owned(0, "parent")?;
            let index: u32 = arg(rest, 1, "index")?.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
            ui(move |ctx| {
                let p = children_of(ctx, resolve_target(ctx, &parent)?)?;
                hr(unsafe { ctx.service.RemoveChild(p, index) }, "RemoveChild")?;
                Ok(format!("removed child {index} of {p:#x}"))
            })
        }
        other => Err(format!("unknown command `{other}`; try `help`")),
    }
}

fn ui(f: impl FnOnce(&Ctx) -> R<String> + Send + 'static) -> R<String> {
    run_on_ui(f)?
}

pub(crate) fn hr(hr: windows_core::HRESULT, what: &str) -> R<()> {
    hr.ok().map_err(|e| format!("{what}: {e}"))
}

pub(crate) fn parse_handle(s: &str) -> R<InstanceHandle> {
    match s.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).map_err(|e| format!("bad handle `{s}`: {e}")),
        None => s.parse().map_err(|e| format!("bad handle `{s}`: {e}")),
    }
}

fn split_nth(sel: &str) -> (&str, usize) {
    match sel.rsplit_once(':') {
        Some((s, n)) if !s.is_empty() && n.parse::<usize>().is_ok() => (s, n.parse().unwrap()),
        _ => (sel, 0),
    }
}

fn type_matches(actual: &str, wanted: &str) -> bool {
    wanted.is_empty() || actual == wanted || actual.ends_with(&format!(".{wanted}"))
}

/// All elements matching `sel` (ignoring any `:N`), in tree order.
pub(crate) fn matches(sel: &str) -> Vec<(InstanceHandle, String)> {
    let tree = lock(&TREE);
    let order = dfs_order(&tree);
    let label = |h: &InstanceHandle| {
        let n = &tree[h];
        if n.name.is_empty() { n.ty.clone() } else { format!("{}#{}", n.ty, n.name) }
    };
    let found: Vec<InstanceHandle> = match sel.split_once('#') {
        Some((ty, name)) => order.into_iter().filter(|h| tree[h].name == name && type_matches(&tree[h].ty, ty)).collect(),
        None => {
            let by_name: Vec<_> = order.iter().copied().filter(|h| tree[h].name == sel).collect();
            if by_name.is_empty() { order.into_iter().filter(|h| type_matches(&tree[h].ty, sel)).collect() } else { by_name }
        }
    };
    found.iter().map(|h| (*h, label(h))).collect()
}

pub(crate) fn resolve_elem(sel: &str) -> R<InstanceHandle> {
    if sel.starts_with("0x") || sel.chars().all(|c| c.is_ascii_digit()) {
        return parse_handle(sel);
    }
    let (base, nth) = split_nth(sel);
    let found = matches(base);
    found
        .get(nth)
        .map(|(h, _)| *h)
        .ok_or_else(|| format!("`{sel}` matched {} element(s)", found.len()))
}

fn find(sel: &str) -> R<String> {
    let (base, _) = split_nth(sel);
    let mut out = String::new();
    for (i, (h, label)) in matches(base).iter().enumerate() {
        let _ = writeln!(out, "{i:>3}  {h:#x}  {label}");
    }
    Ok(out)
}

/// `elem/Prop/Prop`: hop through object-valued properties.
fn resolve_target(ctx: &Ctx, target: &str) -> R<InstanceHandle> {
    let mut parts = target.split('/');
    let mut h = resolve_elem(parts.next().unwrap_or_default())?;
    for prop in parts {
        let p = props::find_prop(&ctx.service, h, prop)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{h:#x} has no property `{prop}`"))?;
        if p.is_null() || !p.is_handle() {
            return Err(format!("{h:#x}.{prop} is not an object (= {})", if p.is_null() { "null" } else { &p.value }));
        }
        h = parse_handle(&p.value)?;
    }
    Ok(h)
}

pub(crate) fn prop_index(ctx: &Ctx, h: InstanceHandle, prop: &str) -> R<u32> {
    // The property chain carries indices; GetPropertyIndex only knows some names (fails on brushes).
    if let Some(p) = props::find_prop(&ctx.service, h, prop).map_err(|e| e.to_string())? {
        return Ok(p.index);
    }
    let name = HSTRING::from(prop);
    let mut index = 0;
    hr(unsafe { ctx.service.GetPropertyIndex(h, PCWSTR(name.as_ptr()), &mut index) }, &format!("GetPropertyIndex({prop})"))?;
    Ok(index)
}

fn infer_type(ctx: &Ctx, h: InstanceHandle, prop: &str) -> R<String> {
    let p = props::find_prop(&ctx.service, h, prop)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{h:#x} has no property `{prop}`"))?;
    if p.value_type == "Windows.Foundation.Object" {
        return Err(format!("{prop} is currently untyped ({}); pass a Type", p.value));
    }
    Ok(p.value_type)
}

pub(crate) fn create_instance(ctx: &Ctx, ty: &str, value: &str) -> R<InstanceHandle> {
    // BSTR is a transparent pointer; an empty BSTR is null, which is what the API wants for "no value".
    let (ty_b, value_b) = (BSTR::from(ty), BSTR::from(value));
    let raw = |b: &BSTR| unsafe { transmute_copy::<BSTR, *const u16>(b) };
    let mut out = 0;
    hr(unsafe { ctx.service.CreateInstance(raw(&ty_b), raw(&value_b), &mut out) }, &format!("CreateInstance({ty}, {value:?})"))?;
    Ok(out)
}

/// AddChild/RemoveChild operate on collections; let panels stand in for their `Children`.
pub(crate) fn children_of(ctx: &Ctx, h: InstanceHandle) -> R<InstanceHandle> {
    match props::find_prop(&ctx.service, h, "Children").map_err(|e| e.to_string())? {
        Some(p) if p.is_handle() && !p.is_null() => parse_handle(&p.value),
        _ => Ok(h),
    }
}

fn collection_count(ctx: &Ctx, collection: InstanceHandle) -> R<u32> {
    let mut n = 0;
    hr(unsafe { ctx.service.GetCollectionCount(collection, &mut n) }, "GetCollectionCount")?;
    Ok(n)
}

/// One line for the daemon: which TAP copy, which taskbar tree, and what we currently show.
fn status() -> R<String> {
    let taskbar = resolve_elem("BackgroundFill").map_or("none".to_string(), |h| format!("{h:#x}"));
    // The UI hop doubles as a liveness check of the taskbar thread.
    let (backdrop, alert, meters, toast) =
        run_on_ui(|_| (crate::backdrop::is_on(), crate::alert::is_on(), crate::meters::count(), crate::toast::is_shown()))?;
    let on = |b: bool| if b { "on" } else { "off" };
    let toast = if toast { "shown" } else { "hidden" };
    Ok(format!("tap={} taskbar={taskbar} backdrop={} alert={} meters={meters} toast={toast}", *crate::TAP_ID, on(backdrop), on(alert)))
}
