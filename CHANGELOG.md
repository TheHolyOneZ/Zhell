# Changelog

All notable changes to Zhell are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [0.1.1] - 2026-10-19

### Fixed

- **Pasting text copied in other apps on Wayland.** Text copied in a browser, chat app or any other native Wayland program could not be pasted into Zhell (Ctrl+Shift+V, Shift+Insert, the right-click menu or middle-click); only text copied inside Zhell worked. Zhell now uses the window's own Wayland connection for the clipboard and the primary selection instead of going through XWayland, so copy and paste work both ways on KDE, GNOME, Hyprland, Sway and other compositors. X11 and Windows are unchanged.

## [0.1.0] - 2026-10-07

- First release.

[0.1.1]: https://github.com/TheHolyOneZ/Zhell/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/TheHolyOneZ/Zhell/releases/tag/v0.1.0
