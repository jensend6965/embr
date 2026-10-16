// Raise Embr's windows above everything else and focus one of them, so `spectacle -a` captures
// the app rather than whatever else happens to be focused. Loaded through KWin's scripting
// D-Bus interface:
//
//   qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript <this file> embr-raise
//   qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start
//
// The About and Settings windows win if they are open: each is a window of its own, and each is
// harder to reach than the main one. Settings beats the main window; About beats everything.

function isEmbr(w) {
    var cls = String(w.resourceClass || "") + " " + String(w.resourceName || "");
    return cls.toLowerCase().indexOf("embr") >= 0;
}

var seen = 0;
var about = null;
var settings = null;
var main = null;
var windows = workspace.windowList();
for (var i = 0; i < windows.length; i++) {
    var w = windows[i];
    if (!isEmbr(w)) {
        continue;
    }
    seen = seen + 1;
    w.keepAbove = true;
    w.minimized = false;
    var caption = String(w.caption || "").toLowerCase();
    if (caption.indexOf("about") >= 0) {
        about = w;
    } else if (caption.indexOf("settings") >= 0) {
        settings = w;
    } else {
        main = w;
    }
}

if (about !== null) {
    workspace.activeWindow = about;
} else if (settings !== null) {
    workspace.activeWindow = settings;
} else if (main !== null) {
    workspace.activeWindow = main;
}

print("embr-raise: " + seen + " window(s); activated "
      + (about !== null ? "about" : (settings !== null ? "settings" : (main !== null ? "main" : "none"))));
