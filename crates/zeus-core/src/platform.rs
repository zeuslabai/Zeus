//! Cross-platform helpers for desktop operations (URL opening, notifications).
//!
//! Each function picks the right native command for macOS, Linux, or Windows.

use std::process::Command;

/// Open a URL in the default browser.
///
/// - macOS: `open <url>`
/// - Linux: `xdg-open <url>`
/// - Windows: `cmd /c start <url>`
pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(url).spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }

    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd").args(["/c", "start", "", url]).spawn();
    }

    // FreeBSD / other — no-op (headless servers)
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
    }
}

/// Send a desktop notification.
///
/// - macOS: osascript `display notification` (title/message via argv, no injection)
/// - Linux: `notify-send` from libnotify-bin
/// - Windows: PowerShell `New-BurntToastNotification` or `BalloonTipText` fallback
///
/// Spawns a reaper thread to avoid zombie processes.
pub fn send_desktop_notification(title: &str, message: &str) {
    let safe_msg: String = message.chars().take(200).collect();

    #[cfg(target_os = "macos")]
    {
        let script = concat!(
            "on run argv\n",
            "  display notification (item 2 of argv) ",
            "with title (item 1 of argv) ",
            "sound name \"Submarine\"\n",
            "end run",
        );
        if let Ok(mut child) = Command::new("osascript")
            .args(["-e", script, "--", title, &safe_msg])
            .spawn()
        {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(mut child) = Command::new("notify-send")
            .args(["--app-name=Zeus", title, &safe_msg])
            .spawn()
        {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    #[cfg(target_os = "windows")]
    {
        // PowerShell toast notification — works on Windows 10+
        let ps_script = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
             $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $text = $template.GetElementsByTagName('text'); \
             $text.Item(0).AppendChild($template.CreateTextNode('{}')) > $null; \
             $text.Item(1).AppendChild($template.CreateTextNode('{}')) > $null; \
             $toast = [Windows.UI.Notifications.ToastNotification]::new($template); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Zeus').Show($toast)",
            title.replace('\'', "''"),
            safe_msg.replace('\'', "''"),
        );
        if let Ok(mut child) = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps_script])
            .spawn()
        {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (title, safe_msg);
    }
}
