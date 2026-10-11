// Injected into Steam's SharedJSContext (cdp.rs); reports one snapshot per topic via
// `__framemateEmit`. Undocumented Steam internals: guard every access, report failures as
// {topic: "error"}. Must be idempotent: it is re-evaluated on every reconnect.
(() => {
  const W = window;
  try { W.__framemate?.dispose?.(); } catch {}

  const handles = [], timers = [], cleanups = [];
  let disposed = false;
  W.__framemate = {
    dispose() {
      disposed = true;
      for (const h of handles) try { h.unregister(); } catch {}
      for (const t of timers) clearInterval(t);
      for (const c of cleanups) try { c(); } catch {}
    },
  };

  const encode = v => JSON.stringify(v, (_, x) => (typeof x === "bigint" ? String(x) : x));
  const emit = (topic, data) => {
    if (disposed) return;
    try { W.__framemateEmit(encode({ topic, data })); } catch {}
  };
  const fail = (topic, e) => emit("error", { topic, message: String(e?.message ?? e) });
  const subscribe = (topic, register, handler) => {
    try {
      const h = register((...args) => {
        try { handler(...args); } catch (e) { fail(topic, e); }
      });
      if (h?.unregister) handles.push(h);
    } catch (e) { fail(topic, e); }
  };
  const poll = (topic, ms, fn) => {
    const run = async () => {
      try {
        const data = await fn();
        if (data !== undefined) emit(topic, data);
      } catch (e) { fail(topic, e); }
    };
    run();
    timers.push(setInterval(run, ms));
  };
  const appName = appid => {
    try { return W.appStore.GetAppOverviewByAppID(Number(appid))?.display_name ?? null; } catch { return null; }
  };

  // --- battery ---
  const battery = () => subscribe("battery", cb => SteamClient.System.RegisterForBatteryStateChanges(cb), s =>
    emit("battery", {
      has_battery: s.bHasBattery,
      level: s.flLevel,
      seconds_remaining: s.nSecondsRemaining,
      ac_state: s.eACState, // 1 = on battery, 2 = on AC (observed)
      battery_state: s.eBatteryState,
    }));

  // --- downloads: this device only (other PCs show up via Remote Downloads) ---
  const isLocal = clientId => String(clientId) === "0";
  const downloads = () => {
    subscribe("downloads", cb => SteamClient.Downloads.RegisterForDownloadItems(cb), (_flag, clients) => {
      const local = (clients ?? []).filter(c => isLocal(c.remote_client_id)).flatMap(c => c.item_data ?? []);
      emit("downloads", local.map(i => ({
        appid: i.appid,
        name: appName(i.appid),
        queue_index: i.queue_index,
        active: i.active,
        paused: i.paused,
        completed: i.completed,
        completed_time: i.completed_time,
        percent: i.overall_percent_complete ?? null,
        error: i.update_error || null,
        buildid: i.buildid,
        target_buildid: i.target_buildid,
      })));
    });
    // ~1 Hz while downloading; may describe a remote client's transfer, reported as idle.
    subscribe("download_overview", cb => SteamClient.Downloads.RegisterForDownloadOverview(cb), o => {
      if (!isLocal(o.remote_client_id)) {
        emit("download_overview", { appid: null });
        return;
      }
      emit("download_overview", {
        appid: o.update_appid || null,
        name: o.update_appid ? appName(o.update_appid) : null,
        state: o.update_state || null,
        paused: o.paused,
        percent: o.overall_percent_complete,
        bytes_per_sec: o.update_network_bytes_per_second,
        peak_bytes_per_sec: o.update_peak_network_bytes_per_second,
        disc_bytes_per_sec: o.update_disc_bytes_per_second,
        eta_sec: Math.max(0, ...(o.progress ?? []).map(p => p.estimated_time_remaining_sec || 0)),
        is_install: o.update_is_install,
        is_workshop: o.update_is_workshop,
        is_shader: o.update_is_shader,
        is_upload: o.update_is_upload,
        throttle_bytes_per_sec: o.download_throttle_rate,
      });
    });
  };

  // --- running apps ---
  const runningApps = () =>
    Array.from(W.SteamUIStore?.RunningApps ?? [], a => ({ appid: a.appid, name: a.display_name ?? appName(a.appid) }));
  const apps = () => {
    poll("running_apps", 5000, runningApps);
    subscribe("last_app_event", cb => SteamClient.GameSessions.RegisterForAppLifetimeNotifications(cb), n => {
      emit("last_app_event", { appid: n.unAppID, name: appName(n.unAppID), running: n.bRunning, at_ms: Date.now(), raw: n });
      setTimeout(() => emit("running_apps", runningApps()), 500);
    });
  };

  // --- VR devices. openvr.h property ids: 1001 ModelNumber, 1002 SerialNumber,
  // 1011 DeviceIsCharging, 1012 DeviceBatteryPercentage, 1029 DeviceClass ---
  let vrPaths = [];
  const vrDevice = async path => {
    const P = SteamClient.OpenVR.DeviceProperties;
    const get = async (fn, id) => { try { return await P[fn](path, id); } catch { return null; } };
    let connected = null;
    try { connected = await SteamClient.OpenVR.Device.BIsConnected(path); } catch {}
    return {
      path,
      model: await get("GetStringDeviceProperty", 1001),
      serial: await get("GetStringDeviceProperty", 1002),
      device_class: await get("GetInt32DeviceProperty", 1029), // 1 = HMD, 2 = controller
      battery: await get("GetFloatDeviceProperty", 1012),
      charging: await get("GetBoolDeviceProperty", 1011),
      connected,
    };
  };
  const vrDevices = () => Promise.all(vrPaths.map(vrDevice));
  const vr = () => {
    subscribe("vr_devices", cb => SteamClient.OpenVR.RegisterForVRTrackedDevices(cb), paths => {
      vrPaths = Array.isArray(paths) ? paths : [];
      vrDevices().then(d => emit("vr_devices", d), e => fail("vr_devices", e));
    });
    poll("vr_devices", 15000, () => (vrPaths.length ? vrDevices() : undefined));

    const state = {};
    subscribe("vr_state", cb => SteamClient.OpenVR.RegisterForHMDActivityLevelChanged(cb), level => {
      state.hmd_activity = level;
      emit("vr_state", { ...state });
    });
    subscribe("vr_state", cb => SteamClient.OpenVR.RegisterForVRSceneAppChange(cb), appid => {
      state.scene_appid = appid || null;
      state.scene_app_name = appid ? appName(appid) : null;
      emit("vr_state", { ...state });
    });
  };

  // --- network, perf, system, user (polled) ---
  const stores = () => {
    poll("network", 10000, () => {
      const n = W.SystemNetworkStore, d = W.SystemPerfStore?.msgDiagnosticInfo;
      return {
        connected: n?.hasNetworkConnection ?? null,
        internet: n?.hasInternetConnection ?? null,
        steam: n?.hasSteamConnection ?? null,
        wifi_enabled: n?.isWifiEnabled ?? null,
        interfaces: (d?.interfaces ?? [])
          .filter(i => i.name !== "lo" && Number(i.rx_bytes_total) > 0)
          .map(i => ({
            name: i.name,
            rx_bytes_per_sec: i.rx_bytes_per_sec,
            tx_bytes_per_sec: i.tx_bytes_per_sec,
            rx_bytes_total: String(i.rx_bytes_total),
            tx_bytes_total: String(i.tx_bytes_total),
          })),
      };
    });
    poll("perf", 15000, () => {
      const p = W.SystemPerfStore, a = p?.msgState?.settings?.per_app ?? {};
      return {
        battery_temp_c: p?.nBatteryTemperatureC ?? null,
        current_game_id: p?.nCurrentGameID != null ? String(p.nCurrentGameID) : null,
        tdp_limit_w: a.is_tdp_limit_enabled ? a.tdp_limit : null,
        fps_limit: a.is_fps_limit_enabled ? a.fps_limit : null,
        low_latency_mode: a.is_low_latency_mode_enabled ?? null,
      };
    });
    poll("system_info", 600000, async () => {
      const s = await SteamClient.System.GetSystemInfo();
      return {
        os: s.sOSName,
        os_version: s.sOSVersionId,
        os_variant: s.sOSVariantId,
        os_build: s.sOSBuildId,
        kernel: s.sKernelVersion,
        hostname: s.sHostname,
        product: s.sProductVendor,
        steam_version: s.nSteamVersion,
        steam_build_date: s.sSteamBuildDate,
        gpu: s.sVideoCardName,
        gpu_driver: s.sVideoDriverVersion,
        cpu_cores: s.nCPULogicalCores,
        ram_mb: s.nSystemRAMSizeMB,
      };
    });
    poll("user", 60000, () => {
      const u = W.App?.m_CurrentUser, p = W.g_FriendsUIApp?.FriendStore?.self?.persona;
      return {
        account_name: u?.strAccountName ?? null,
        persona_name: p?.m_strPlayerName ?? null,
        persona_state: p?.m_ePersonaState ?? null, // 0 offline, 1 online, 2 busy, 3 away, ...
        offline_mode: u?.bIsOfflineMode ?? null,
      };
    });
  };

  // --- notifications in the headset: Steam's own toast queue, rendered by our component ---
  // Steam has no toast type for free text (each type has a fixed renderer), so creating the
  // element for its renderer is intercepted for notifications that carry `framemate`. Steam
  // still does the rest: queue, sound, the SteamVR notification overlay ("SteamVR -
  // Notifications"), "disable all toasts" and suppression while the headset is asleep.
  // Webpack modules are found by shape, never by id (ids change with every Steam build).
  const LOGO = "data:image/svg+xml;charset=utf-8," + encodeURIComponent("__FRAMEMATE_LOGO_SVG__");
  const toasts = () => {
    let require;
    W.webpackChunksteamui.push([[Symbol("framemate")], {}, r => { require = r; }]);
    // Steams runtime exposes no module cache (`require.c`), require every id (like decky) also evaluate modules Steam hadnt run yet
    const loaded = Object.keys(require?.m ?? {})
      .map(id => { try { return require(id); } catch { return null; } })
      .filter(e => e && typeof e === "object");
    const source = f => { try { return Function.prototype.toString.call(f); } catch { return ""; } };
    let renderer = null;
    for (const e of loaded) {
      for (const key of Object.keys(e)) {
        let v;
        try { v = e[key]; } catch { continue; }
        if (typeof v === "function" && source(v).includes('controller:"notification",method:')) renderer = v;
      }
    }
    const runtimes = loaded.filter(e => typeof e.jsx === "function" && typeof e.jsxs === "function" && "Fragment" in e);
    const css = loaded.map(e => (e.ShortTemplate ? e : e.default)).find(e => e?.ShortTemplate && e.ShortLogoDimensions && e.Title && e.Body);
    // toasts borrow "download complete" type. must be a real type
    const store = Object.getPrototypeOf(W.NotificationStore);
    const member = f => /\.OnNotification\([^,]+,[\w$]+\.[\w$]+\.([\w$]+),/.exec(source(f))?.[1];
    const typeKey = member(store.TestDownloadComplete), checkKey = member(store.TestFriendMessage);
    let type;
    for (const e of typeKey && checkKey ? loaded : []) {
      for (const key of Object.keys(e)) {
        let v;
        try { v = e[key]; } catch { continue; }
        if (v && typeof v === "object" && Number.isInteger(v[typeKey]) && Number.isInteger(v[checkKey])) type ??= v[typeKey];
      }
    }
    if (!renderer || !runtimes.length || !css || type === undefined) {
      throw new Error(`toast hook: renderer ${!!renderer}, jsx runtimes ${runtimes.length}, css ${!!css}, type ${type}`);
    }

    const { jsx, jsxs } = runtimes[0]; // originals, taken before patching
    const cx = (...names) => names.filter(Boolean).join(" ");
    // Same markup and classes as Steam's own short toast template.
    const Toast = ({ group, onDismiss }) => {
      const n = group.notifications[group.notifications.length - 1].framemate;
      return jsxs("div", {
        className: cx(css.ShortTemplate, n.body && css.TwoLine),
        onClick: () => onDismiss?.(),
        children: [
          jsx("div", {
            className: css.ShortLogoDimensions,
            children: jsx("img", { src: n.icon || LOGO, style: { width: "100%", height: "100%", objectFit: "contain" } }),
          }),
          jsxs("div", {
            className: css.Content,
            children: [
              jsx("div", { className: css.Header, children: jsx("div", { className: css.Title, children: n.title }) }),
              n.body ? jsx("div", { className: css.Body, children: n.body }) : null,
            ],
          }),
        ],
      });
    };

    for (const runtime of runtimes) {
      for (const name of ["jsx", "jsxs"]) {
        const original = runtime[name];
        const wrapper = function (type, props) {
          if (!disposed && type === renderer && props?.group?.notifications?.[0]?.framemate) {
            return original.call(this, Toast, props, ...[...arguments].slice(2));
          }
          return original.apply(this, arguments);
        };
        runtime[name] = wrapper;
        // If something wrapped it again on top of us, `disposed` turns ours into a passthrough.
        cleanups.push(() => { if (runtime[name] === wrapper) runtime[name] = original; });
      }
    }

    const text = (v, max) => String(v ?? "").slice(0, max);
    // Only images: arbitrary URL would let a caller make Steams UI fetch it
    const image = v => (typeof v === "string" && /^data:image\/(png|jpeg|webp|svg\+xml)[;,]/.test(v) ? v : null);
    // sound follows borrowed types config and users toast sound setting
    // returns {id, suppressed}; suppressed = the headset is asleep, so Steam drops the toast
    W.__framemate.notify = ({ title, body, icon, duration_ms = 6000 } = {}) => {
      const store = W.NotificationStore;
      const id = store.m_nNextTestNotificationID++;
      const suppressed = !!store.BShouldSupressToasts();
      store.ProcessNotification(
        // bEnableInReducedUI: the Frame runs Steam in "reduced UI" mode, which drops other toasts
        { showToast: true, playSound: true, sound: 6 /* ToastMisc */, eFeature: 0, bCritical: false, fnTray: null,
          bEnableInReducedUI: true,
          toastDurationMS: Math.min(Math.max(Number(duration_ms) || 6000, 2000), 15000) },
        { notificationID: id, rtCreated: Math.floor(Date.now() / 1000), eType: type, nToastDurationMS: 0,
          eSource: 1, data: {}, bNewIndicator: true,
          framemate: { title: text(title, 120), body: text(body, 500), icon: image(icon) } },
        0 /* New */);
      return { id, suppressed };
    };
  };

  // Steam's stores load after the context is created; wait for them.
  const start = () => {
    if (disposed) return;
    if (typeof SteamClient === "undefined" || !W.appStore || !W.SteamUIStore || !W.NotificationStore || !W.webpackChunksteamui) {
      timers.push(setTimeout(start, 2000));
      return;
    }
    for (const section of [battery, downloads, apps, vr, stores, toasts]) {
      try { section(); } catch (e) { fail(section.name, e); }
    }
  };
  start();
  return "framemate shim installed";
})()
