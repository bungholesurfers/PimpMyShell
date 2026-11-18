# Security

PimpMyShell runs a DLL inside explorer.exe, and reads your notifications to draw them. A hole in it is worth knowing
about.

## Reporting one

Privately, please, not in a public issue. On GitHub: **Security > Report a vulnerability** on this repository, or
straight to <https://github.com/bungholesurfers/PimpMyShell/security/advisories/new>. Only the maintainer sees what you
write there.

Say what you found, in which version, and how to see it happen. The answer comes in the same place. Once a release has
the fix, its notes say what it was, and who found it if you'd like that.

## What gets fixed

The latest release. Older ones don't get fixes of their own: PimpMyShell updates itself, or tells you there's a newer
one.

## What counts

Anything that lets something else, on the PC or on the network, do through PimpMyShell what it couldn't do otherwise.
For instance:

- reading what the daemon and its part in Explorer (the TAP) say to each other, or getting either to act for it;
- getting Explorer to load something that isn't PimpMyShell's;
- a helper getting out of its sandbox (the Twatch player, Streamlink, the PIMPBOX's game, pms-fetch), into the daemon,
  Explorer or anywhere else: by what it hands over (a frame, a line it writes, the video surface Explorer shows) or
  otherwise;
- getting an update installed that isn't ours;
- getting a file written or deleted that isn't PimpMyShell's own.

What doesn't: what a program can do to PimpMyShell that already runs as you, with all of your rights. It can do that
to anything of yours.
