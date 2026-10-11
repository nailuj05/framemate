// Phone notifications as toasts in the headset (Android only). The settings live on the Kotlin
// side (NotificationPrefs.kt) because the listener service runs without the WebView; this
// mirrors them for the top bar's bell and the Settings page.

export interface NotificationSettings {
  /** The bell: off by default, and quick to switch off when someone else wears the headset. */
  enabled: boolean;
  /** Apps that posted a notification so far; all forwarded unless switched off. */
  apps: { pkg: string; label: string; enabled: boolean }[];
  /** Outcome of the last send: shown, headset_asleep, rate_limited, not_paired, error: … */
  last: { result: string; time: number } | null;
}

const bridge = () => window.FrameMateAndroid;

class Notifications {
  readonly supported = typeof window !== "undefined" && !!window.FrameMateAndroid?.notificationSettings;
  settings = $state<NotificationSettings | null>(null);
  access = $state(false);
  testing = $state(false);

  get enabled() {
    return this.settings?.enabled ?? false;
  }

  refresh() {
    if (!this.supported) return;
    try {
      this.settings = JSON.parse(bridge()!.notificationSettings!());
      this.access = bridge()!.notificationAccessGranted?.() ?? false;
    } catch {}
  }

  setEnabled(enabled: boolean) {
    bridge()?.setNotificationsEnabled?.(enabled);
    this.refresh();
  }

  setApp(pkg: string, enabled: boolean) {
    bridge()?.setNotificationAppEnabled?.(pkg, enabled);
    this.refresh();
  }

  openAccessSettings() {
    bridge()?.openNotificationAccessSettings?.();
  }

  async sendTest() {
    const before = this.settings?.last?.time ?? 0;
    this.testing = true;
    bridge()?.sendTestNotification?.();
    // The send runs on a Kotlin thread (up to ~15 s); poll for its result.
    for (let i = 0; i < 40 && (this.settings?.last?.time ?? 0) <= before; i++) {
      await new Promise(resolve => setTimeout(resolve, 500));
      this.refresh();
    }
    this.testing = false;
  }
}

export const notifications = new Notifications();

export function describeResult(result: string) {
  return (
    {
      shown: "Shown in the headset.",
      headset_asleep: "Delivered, but the headset is asleep, so Steam didn't show it.",
      rate_limited: "Too many at once. Try again in a few seconds.",
      not_paired: "Pair the app with your Frame first.",
      disabled: "Notifications are off. Turn them on with the bell in the top bar.",
    }[result] ?? result.replace(/^error: /, "Failed: ")
  );
}
