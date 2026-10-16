// Make Embr's window fill the screen, so a layout can be photographed at a width the user reaches by
// maximising — the case a tiled 1262x958 window never shows.
//
//   qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$PWD/scripts/dev/kwin-embr-fullscreen.js" embr-full-$$
//   qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start
//
// Why a script and not a window rule: assigning `frameGeometry` through this KWin fork's scripting API
// silently does nothing (measured), while `fullScreen` is honoured — the app logs
// `[app] window 2560x1080 (physical)` right after. Load it under a **fresh name every time**: KWin
// keeps the previous run loaded and reloading the same name does not re-run it.
//
// The app must be killed afterwards; the window's fullscreen state goes with it.
//
// Note for a *GTK* app: this matches on the window class containing "embr" lowercased, which an app id
// like `io.github.jensend6965.EmbrSpike` satisfies as well.

function isEmbr(w) {
    var cls = String(w.resourceClass || "") + " " + String(w.resourceName || "");
    return cls.toLowerCase().indexOf("embr") >= 0;
}

var windows = workspace.windowList();
var target = null;
for (var i = 0; i < windows.length; i++) {
    var w = windows[i];
    if (!isEmbr(w)) {
        continue;
    }
    w.keepAbove = true;
    w.minimized = false;
    if (String(w.caption || "").toLowerCase().indexOf("about") < 0) {
        target = w;
    }
}

if (target === null) {
    print("embr-fullscreen: no window");
} else {
    print("embr-fullscreen: frame before " + JSON.stringify(target.frameGeometry));
    try { target.fullScreen = true; } catch (e) { print("embr-fullscreen: fullScreen failed: " + e); }
    try { target.maximized = true; } catch (e) { print("embr-fullscreen: maximized failed: " + e); }
    print("embr-fullscreen: frame after " + JSON.stringify(target.frameGeometry));
    workspace.activeWindow = target;
}
