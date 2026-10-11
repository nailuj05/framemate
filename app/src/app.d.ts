declare global {
  namespace App {
    interface PageState {
      /** Headset view in fullscreen (shallow route, so Back leaves fullscreen). */
      fullscreen?: boolean;
    }
  }

  interface Window {
    /** Native bridge from MainActivity.kt (Android only). */
    FrameMateAndroid?: {
      setFullscreen(enabled: boolean): void;
      openUrl(url: string): void;
      /** False on Android 17+ until "Nearby devices" (local network) is granted. */
      localNetworkAllowed?(): boolean;
      openAppSettings?(): void;
      /** Phone notifications. */
      notificationAccessGranted?(): boolean;
      openNotificationAccessSettings?(): void;
      /** JSON, see NotificationSettings in settings/+page.svelte. */
      notificationSettings?(): string;
      setNotificationsEnabled?(enabled: boolean): void;
      setNotificationAppEnabled?(pkg: string, enabled: boolean): void;
      /** Returns at once; the outcome shows up in notificationSettings().last. */
      sendTestNotification?(): void;
    };
  }
}

export {};
