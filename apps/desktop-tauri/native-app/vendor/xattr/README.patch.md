# xattr 0.2.3 patch

`zed-async-tar`, reached through `gpui_http_client`, pins `xattr = "0.2"`.
Version 0.2.3 defines `ENOATTR` from `libc::ENOATTR`, which current libc no
longer exposes on Linux (the same errno is named `ENODATA`). This copy changes
only that constant, keeping the public API and version unchanged.
