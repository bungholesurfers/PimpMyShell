# The updater's pointer

`stable.txt` holds the tag of the release PimpMyShell's updater moves copies to. It's read from the `main` branch
(`https://raw.githubusercontent.com/bungholesurfers/PimpMyShell/main/update/stable.txt`, see `pms/src/update.rs`), and
when it names a version newer than a copy's own, that copy fetches the release's signed `pms-update.json`, then its
installer.

To roll a release out:

1. Push its tag; the release workflow builds it and drafts the release, with `pms-update.json` and its signature
   (the workflow needs the secret `PMS_UPDATE_KEY` for that; `installer/INSTALLER.md` says how to make it).
2. Publish the release, and try it.
3. When you're happy with it, put its tag here (`v0.1.1`, say) and push.

Setting an older tag holds updates back. It never moves anyone backwards: PimpMyShell only installs a version newer
than its own.