//! What the notification platform holds right now, side by side: the listener's view (what the
//! daemon polls) and the notification database (`wpndatabase.db`, where payloads come from).
//!
//!   notif_probe                 the newest 15 database rows, then the listener's toasts
//!   notif_probe schema          the database's tables and columns
//!   notif_probe settings AUMID  that app's Notifications\Settings values
//!   notif_probe apps [list]     how many apps the AppsFolder holds (and their ids, with a second argument)
//!   notif_probe xml ID          that row's payload
//!   notif_probe row ID..        those rows' columns, payload aside
//!   notif_probe transient       the newest 15 TransientTable rows

use windows::UI::Notifications::Management::UserNotificationListener;
use windows::UI::Notifications::NotificationKinds;

fn db() -> rusqlite::Connection {
    let path = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join(r"Microsoft\Windows\Notifications\wpndatabase.db");
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
}

fn schema() {
    let db = db();
    let mut q = db.prepare("select name, sql from sqlite_master where type = 'table'").unwrap();
    for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))).unwrap().flatten() {
        println!("{}\n  {}\n", row.0, row.1.unwrap_or_default());
    }
}

fn settings(aumid: &str) {
    let out = std::process::Command::new("reg")
        .args(["query", &format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{aumid}")])
        .output()
        .unwrap();
    print!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("schema") => return schema(),
        Some("settings") => return settings(&args[1]),
        Some("apps") => {
            use windows::Win32::System::Com::*;
            use windows::Win32::UI::Shell::*;
            let t = std::time::Instant::now();
            let mut n = 0;
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
                let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None).unwrap();
                let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems).unwrap();
                loop {
                    let mut got = [None];
                    if items.Next(&mut got, None).is_err() || got[0].is_none() {
                        break;
                    }
                    let item = got[0].take().unwrap();
                    let id = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).unwrap();
                    if args.get(1).is_some() {
                        println!("  {}", id.display());
                    }
                    CoTaskMemFree(Some(id.0 as _));
                    n += 1;
                }
            }
            println!("{n} apps in {:?}", t.elapsed());
            return;
        }
        Some("xml") => {
            let bytes: Vec<u8> = db().query_row("select Payload from Notification where Id = ?1", [&args[1]], |r| r.get(0)).unwrap();
            println!("{}", String::from_utf8_lossy(&bytes));
            return;
        }
        Some("row") => {
            let db = db();
            for id in &args[1..] {
                let mut q = db.prepare("select * from Notification where Id = ?1").unwrap();
                let names: Vec<String> = q.column_names().iter().map(|s| s.to_string()).collect();
                let _ = q.query_row([id], |r| {
                    for (i, name) in names.iter().enumerate() {
                        if name != "Payload" {
                            println!("  {id} {name} = {:?}", r.get::<_, rusqlite::types::Value>(i)?);
                        }
                    }
                    Ok(())
                });
                println!();
            }
            return;
        }
        Some("transient") => {
            let db = db();
            let mut q = db.prepare("select NotificationId, SuppressPopup, Priority, IsMirroringDisabled from TransientTable order by NotificationId desc limit 15").unwrap();
            for row in q
                .query_map([], |r| Ok(format!("  {} suppress={:?} priority={:?} nomirror={:?}", r.get::<_, i64>(0)?, r.get::<_, rusqlite::types::Value>(1)?, r.get::<_, rusqlite::types::Value>(2)?, r.get::<_, rusqlite::types::Value>(3)?)))
                .unwrap()
            {
                println!("{}", row.unwrap_or_else(|e| e.to_string()));
            }
            return;
        }
        _ => {}
    }
    let db = db();
    println!("database, newest first:");
    let mut q = db
        .prepare(
            "select n.Id, h.PrimaryId, n.Type, n.Tag, n.\"Group\", n.ArrivalTime, n.ExpiryTime, length(n.Payload), substr(n.Payload, 1, 160) \
             from Notification n left join NotificationHandler h on h.RecordId = n.HandlerId order by n.Id desc limit 15",
        )
        .unwrap();
    let rows = q
        .query_map([], |r| {
            Ok(format!(
                "  {} {} type={} tag={:?} group={:?} arrival={} expiry={} len={}\n      {}",
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(5)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(6)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(7)?.unwrap_or_default(),
                String::from_utf8_lossy(&r.get::<_, Option<Vec<u8>>>(8)?.unwrap_or_default()).replace('\n', " "),
            ))
        })
        .unwrap();
    for row in rows {
        match row {
            Ok(s) => println!("{s}"),
            Err(e) => println!("  (row: {e})"),
        }
    }

    println!("\nlistener (what the daemon polls):");
    let l = UserNotificationListener::Current().unwrap();
    println!("  access: {:?}", l.GetAccessStatus().unwrap());
    let list = l.GetNotificationsAsync(NotificationKinds::Toast).unwrap().join().unwrap();
    for n in list {
        let info = n.AppInfo().ok();
        let aumid = info.as_ref().and_then(|i| i.AppUserModelId().ok()).map(|s| s.to_string()).unwrap_or_default();
        let name = info.as_ref().and_then(|i| i.DisplayInfo().ok()).and_then(|d| d.DisplayName().ok()).map(|s| s.to_string()).unwrap_or_default();
        println!("  {} {aumid} ({name})", n.Id().unwrap_or(0));
    }
}
