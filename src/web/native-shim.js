(function() {
    console.debug('[Media] Installing native shim...');

    // This branded client has exactly one server. Jellyfin normally sends a
    // signed-out user to its server picker; reconnect to the current server
    // instead so its normal username/password page is shown.
    const isServerPickerRoute = () =>
        /(?:^|[\/#])selectserver(?:\.html)?(?:[/?#]|$)/i.test(
            window.location.pathname + window.location.search + window.location.hash
        );
    const reconnectToCurrentServer = () => {
        if (!isServerPickerRoute()) return;

        const path = window.location.pathname;
        const webIndex = path.toLowerCase().indexOf('/web/');
        const serverPath = webIndex >= 0 ? path.slice(0, webIndex + 1) : '/';
        const serverUrl = window.location.origin + serverPath;
        console.info('[Media] Server picker blocked; reconnecting to:', serverUrl);
        window.location.replace(serverUrl);
    };
    const scheduleServerReconnect = () => setTimeout(reconnectToCurrentServer, 0);
    window.addEventListener('hashchange', scheduleServerReconnect);
    window.addEventListener('popstate', scheduleServerReconnect);
    for (const method of ['pushState', 'replaceState']) {
        const original = window.history[method];
        window.history[method] = function(...args) {
            const result = original.apply(this, args);
            scheduleServerReconnect();
            return result;
        };
    }
    reconnectToCurrentServer();

    // Fullscreen state tracking via HTML5 Fullscreen API
    window._isFullscreen = false;

    document.addEventListener('fullscreenchange', () => {
        const fullscreen = !!document.fullscreenElement;
        if (window._isFullscreen === fullscreen) return;
        window._isFullscreen = fullscreen;
        console.log('[Media] Fullscreen changed:', fullscreen);
        // Notify player so UI updates (jellyfin-web listens for this)
        const player = window._mpvVideoPlayerInstance;
        if (player && player.events) {
            player.events.trigger(player, 'fullscreenchange');
        }
    });

    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && window._isFullscreen) {
            window.jmpNative.toggleFullscreen();
        }
    });

    // Double-click on video area toggles fullscreen.
    // Detected in JS because Wayland doesn't provide click count natively.
    (function() {
        let lastTime = 0, lastX = 0, lastY = 0;
        document.addEventListener('mousedown', (e) => {
            // left button only and only if clicked on main content (not header,
            // or controls)
            if (e.button !== 0 || !e.target.classList.contains("mainAnimatedPage")) return;
            const now = Date.now();
            const dx = e.clientX - lastX;
            const dy = e.clientY - lastY;
            if ((now - lastTime) < 500 && (dx * dx + dy * dy) < 25) {
                if (document.querySelector('.videoPlayerContainer')) {
                    if (window.jmpNative) window.jmpNative.toggleFullscreen();
                }
                lastTime = 0;
            } else {
                lastTime = now;
                lastX = e.clientX;
                lastY = e.clientY;
            }
        }, true);  // capture phase — before jellyfin-web can stopPropagation
    })();

    // Buffered ranges storage (updated by native code)
    window._bufferedRanges = [];
    window._nativeUpdateBufferedRanges = function(ranges) {
        window._bufferedRanges = ranges || [];
    };

    // Signal emulation (Qt-style connect/disconnect)
    function createSignal(name) {
        const callbacks = [];
        const signal = function(...args) {
            for (const cb of callbacks) {
                try { cb(...args); } catch(e) { console.error('[Media] [Signal] ' + name + ' error:', e); }
            }
        };
        signal.connect = (cb) => {
            callbacks.push(cb);
            console.debug('[Media] [Signal] ' + name + ' connected, now has', callbacks.length, 'listeners');
        };
        signal.disconnect = (cb) => {
            const idx = callbacks.indexOf(cb);
            if (idx >= 0) callbacks.splice(idx, 1);
            console.debug('[Media] [Signal] ' + name + ' disconnected, now has', callbacks.length, 'listeners');
        };
        return signal;
    }

    // Saved settings from native (injected as placeholder, replaced at load time)
    const _savedSettings = JSON.parse('__SETTINGS_JSON__');

    // Older builds could render the effective "auto" value while receiving
    // no usable choices for the select menu. Keep a platform-appropriate
    // fallback in the web layer so the control remains functional even if an
    // older/native settings payload omits hwdecOptions.
    const _platform = String(navigator.userAgentData?.platform || navigator.platform || '').toLowerCase();
    const _fallbackHwdecOptions = _platform.includes('win')
        ? ['auto', 'no', 'd3d11va', 'nvdec-copy', 'vulkan']
        : _platform.includes('mac')
            ? ['auto', 'no', 'videotoolbox', 'vulkan']
            : ['auto', 'no', 'vaapi', 'nvdec', 'vulkan'];
    const _nativeHwdecOptions = Array.isArray(_savedSettings.hwdecOptions)
        ? _savedSettings.hwdecOptions.filter(value => typeof value === 'string' && value.length > 0)
        : [];
    const _hwdecOptions = (_nativeHwdecOptions.length ? _nativeHwdecOptions : _fallbackHwdecOptions)
        .map(value => ({
            value,
            title: {
                auto: 'Auto (hardware first)',
                no: 'Disabled (software)',
                d3d11va: 'D3D11VA',
                nvdec: 'NVIDIA NVDEC',
                'nvdec-copy': 'NVIDIA NVDEC (copy-back)',
                vaapi: 'VA-API',
                videotoolbox: 'VideoToolbox',
                vulkan: 'Vulkan'
            }[value] || value
        }));
    const _hwdecValues = new Set(_hwdecOptions.map(option => option.value));
    const _hwdecHelp = _platform.includes('win')
        ? 'Auto is recommended and normally selects zero-copy D3D11VA. NVDEC copy-back still decodes in hardware but can appear as 3D/copy activity. Changes apply to the next video.'
        : 'Auto tries hardware decoding first and falls back to software when needed. Changes apply to the next video.';
    function canonicalHwdec(value) {
        let canonical = String(value == null ? 'auto' : value);
        if (_platform.includes('win') && canonical === 'nvdec') canonical = 'nvdec-copy';
        return _hwdecValues.has(canonical) ? canonical : 'auto';
    }
    const _initialHwdec = canonicalHwdec(_savedSettings.hwdec || 'auto');

    // window.jmpInfo - settings and device info
    window.jmpInfo = {
        version: '__APP_VERSION__',
        deviceName: _savedSettings.deviceName || _savedSettings.deviceNameDefault,
        mode: 'desktop',
        userAgent: navigator.userAgent,
        scriptPath: '',
        sections: [
            { key: 'playback', order: 0 },
            { key: 'audio', order: 1 },
            { key: 'transcode', order: 2 },
            { key: 'advanced', order: 3 }
        ],
        settings: {
            main: { enableMPV: true, fullscreen: false, userWebClient: window.location.origin },
            playback: {
                hwdec: _initialHwdec
            },
            audio: {
                audioPassthrough: _savedSettings.audioPassthrough || '',
                audioExclusive: _savedSettings.audioExclusive || false,
                audioChannels: _savedSettings.audioChannels || ''
            },
            transcode: {
                forceTranscoding: !!_savedSettings.forceTranscoding
            },
            advanced: {
                transparentTitlebar: _savedSettings.transparentTitlebar !== false,
                windowDecorations: __WINDOW_DECORATIONS__,
                hideScrollbar: _savedSettings.hideScrollbar !== false,
                logLevel: _savedSettings.logLevel || '',
                deviceName: _savedSettings.deviceName || ''
            }
        },
        settingsDescriptions: {
            playback: [
                { key: 'hwdec', displayName: 'Hardware Decoding', help: _hwdecHelp, options: _hwdecOptions }
            ],
            audio: [
                { key: 'audioPassthrough', displayName: 'Audio Passthrough', help: 'Comma-separated list of codecs to pass through to the audio device (e.g. ac3,eac3,dts-hd,truehd). Leave empty to disable.', inputType: 'textarea' },
                { key: 'audioExclusive', displayName: 'Exclusive Audio Output', help: 'Take exclusive control of the audio device during playback. May reduce latency but prevents other apps from playing audio.' },
                { key: 'audioChannels', displayName: 'Audio Channel Layout', help: 'Force a specific channel layout. Leave empty for auto-detection.', options: [
                    { value: '', title: 'Auto' },
                    { value: 'stereo', title: 'Stereo' },
                    { value: '5.1', title: '5.1 Surround' },
                    { value: '7.1', title: '7.1 Surround' }
                ]}
            ],
            transcode: [
                { key: 'forceTranscoding', displayName: 'Force Transcoding', help: 'Always request a transcoded stream from the server, even when direct play would work.' }
            ],
            advanced: [
                { key: 'hideScrollbar', displayName: 'Hide Scrollbar', help: 'Hide scrollbars throughout the app. Scrolling with the wheel, trackpad, and keyboard still works. Requires restart.' },
                { key: 'deviceName', displayName: 'Device Name', help: 'Identifies this machine to the server. Leave blank to use the system hostname.', inputType: 'text', maxLength: 64, placeholder: _savedSettings.deviceNameDefault },
                { key: 'logLevel', displayName: 'Log Level', help: 'Set the application log verbosity level.', options: [
                    { value: '', title: 'Default (Info)' },
                    { value: 'verbose', title: 'Verbose' },
                    { value: 'debug', title: 'Debug' },
                    { value: 'warn', title: 'Warning' },
                    { value: 'error', title: 'Error' }
                ]}
            ]
        },
        settingsUpdate: [],
        settingsDescriptionsUpdate: []
    };

    // macOS-only: transparent titlebar toggle (shown first in Advanced section)
    if (navigator.platform.startsWith('Mac')) {
        jmpInfo.settingsDescriptions.advanced.unshift({
            key: 'transparentTitlebar',
            displayName: 'Transparent Titlebar',
            help: 'Overlay traffic light buttons on the window content instead of a separate titlebar. Requires restart.'
        });
    }

    const decorationValues = __WINDOW_DECORATION_OPTIONS__;
    if (decorationValues.length > 1) {
        const decorationTitles = {
            csd: 'In-app (client-side)',
            server: 'System (server-side)',
            serverThemed: 'System, themed (KDE)'
        };
        jmpInfo.settingsDescriptions.advanced.unshift({
            key: 'windowDecorations',
            displayName: 'Window Decorations',
            help: 'How the window titlebar is drawn. Changing requires restart.',
            options: [
                { value: null, title: 'Auto' },
                ...decorationValues.map((value) => ({ value, title: decorationTitles[value] || value }))
            ]
        });
    }

    // Player state
    const playerState = {
        position: 0,
        duration: 0,
        volume: 100,
        muted: false,
        paused: false
    };

    // window.api.player - MPV control API
    window.api = {
        player: {
            // Signals (Qt-style)
            playing: createSignal('playing'),
            paused: createSignal('paused'),
            finished: createSignal('finished'),
            stopped: createSignal('stopped'),
            canceled: createSignal('canceled'),
            error: createSignal('error'),
            buffering: createSignal('buffering'),
            seeking: createSignal('seeking'),
            positionUpdate: createSignal('positionUpdate'),
            updateDuration: createSignal('updateDuration'),
            stateChanged: createSignal('stateChanged'),
            videoPlaybackActive: createSignal('videoPlaybackActive'),
            windowVisible: createSignal('windowVisible'),
            onVideoRecangleChanged: createSignal('onVideoRecangleChanged'),
            onMetaData: createSignal('onMetaData'),

            // Methods
            load(url, options, streamdata, videoStream, audioStream, subtitleStream, externalAudioUrl, externalSubUrl, callback) {
                console.debug('[Media] player.load:', url);
                if (callback) {
                    // Wait for playing signal before calling callback
                    const onPlaying = () => {
                        this.playing.disconnect(onPlaying);
                        this.error.disconnect(onError);
                        callback();
                    };
                    const onError = () => {
                        this.playing.disconnect(onPlaying);
                        this.error.disconnect(onError);
                        callback();
                    };
                    this.playing.connect(onPlaying);
                    this.error.connect(onError);
                }
                if (window.jmpNative && window.jmpNative.playerLoad) {
                    const metadataJson = streamdata?.metadata ? JSON.stringify(streamdata.metadata) : '{}';
                    window.jmpNative.playerLoad(url, options.startMilliseconds, videoStream, audioStream, subtitleStream, metadataJson, externalAudioUrl || '', externalSubUrl || '', !!options.isInfiniteStream);
                }
            },
            stop() {
                console.debug('[Media] player.stop');
                if (window.jmpNative) window.jmpNative.playerStop();
            },
            pause() {
                console.debug('[Media] player.pause');
                if (window.jmpNative) window.jmpNative.playerPause();
                playerState.paused = true;
            },
            play() {
                console.debug('[Media] player.play');
                if (window.jmpNative) window.jmpNative.playerPlay();
                playerState.paused = false;
            },
            seekTo(ms) {
                console.debug('[Media] player.seekTo:', ms);
                if (window.jmpNative) window.jmpNative.playerSeek(ms);
            },
            setVolume(vol) {
                console.debug('[Media] player.setVolume:', vol);
                playerState.volume = vol;
                if (window.jmpNative) window.jmpNative.playerSetVolume(vol);
            },
            setMuted(muted) {
                console.debug('[Media] player.setMuted:', muted);
                playerState.muted = muted;
                if (window.jmpNative) window.jmpNative.playerSetMuted(muted);
            },
            setPlaybackRate(rate) {
                console.debug('[Media] player.setPlaybackRate:', rate);
                if (window.jmpNative) window.jmpNative.playerSetSpeed(rate);
            },
            setSubtitleStream(index) {
                console.debug('[Media] player.setSubtitleStream:', index);
                if (window.jmpNative) window.jmpNative.playerSetSubtitle(index);
            },
            addSubtitleStream(url) {
                console.debug('[Media] player.addSubtitleStream:', url);
                if (window.jmpNative) window.jmpNative.playerAddSubtitle(url);
            },
            setAudioStream(index) {
                console.debug('[Media] player.setAudioStream:', index);
                if (window.jmpNative) window.jmpNative.playerSetAudio(index);
            },
            addAudioStream(url) {
                console.debug('[Media] player.addAudioStream:', url);
                if (window.jmpNative) window.jmpNative.playerAddAudio(url);
            },
            setSubtitleDelay(ms) {
                console.debug('[Media] player.setSubtitleDelay:', ms);
                if (window.jmpNative) window.jmpNative.playerSetSubtitleDelay(ms / 1000.0);
            },
            setAudioDelay(ms) {
                console.debug('[Media] player.setAudioDelay:', ms);
                if (window.jmpNative) window.jmpNative.playerSetAudioDelay(ms / 1000.0);
            },
            setAspectMode(mode) {
                console.debug('[Media] player.setAspectMode:', mode);
                if (window.jmpNative) window.jmpNative.playerSetAspectMode(mode);
            },
            setVideoRectangle(x, y, w, h) {
                // No-op for now, we always render fullscreen
            },
            getPosition(callback) {
                if (callback) callback(playerState.position);
                return playerState.position;
            },
            getDuration(callback) {
                if (callback) callback(playerState.duration);
                return playerState.duration;
            },
        },
        system: {
            openExternalUrl(url) {
                window.open(url, '_blank');
            },
            exit() {
                if (window.jmpNative) window.jmpNative.appExit();
            },
            cancelServerConnectivity() {
                if (window.jmpCheckServerConnectivity && window.jmpCheckServerConnectivity.abort) {
                    window.jmpCheckServerConnectivity.abort();
                }
            }
        },
        settings: {
            setValue(section, key, value, callback) {
                if (window.jmpNative && window.jmpNative.setSettingValue) {
                    let serialized;
                    if (value === null)                  serialized = null;
                    else if (typeof value === 'boolean') serialized = value ? 'true' : 'false';
                    else if (Array.isArray(value))       serialized = JSON.stringify(value);
                    else                                 serialized = String(value);
                    window.jmpNative.setSettingValue(section, key, serialized);
                }
                if (callback) callback();
            },
            sectionValueUpdate: createSignal('sectionValueUpdate'),
            groupUpdate: createSignal('groupUpdate')
        },
        input: {
            // Signals for media session control commands
            hostInput: createSignal('hostInput'),
            positionSeek: createSignal('positionSeek'),
            rateChanged: createSignal('rateChanged'),
            volumeChanged: createSignal('volumeChanged'),

            executeActions() {}
        }
    };

    let pictureInPictureCloseHost = null;
    let pictureInPictureHost = null;
    let pictureInPictureButton = null;
    let videoOsdVisible = false;
    let videoPlaybackActive = false;
    let pictureInPictureMenuRequested = false;
    let pictureInPictureMenuOpened = false;

    // Expose signal emitter for native code
    window._nativeEmit = function(signal, ...args) {
        console.debug('[Media] _nativeEmit called with signal:', signal, 'args:', args);
        if (signal === 'playing') {
            videoPlaybackActive = true;
        } else if (signal === 'finished' || signal === 'canceled' || signal === 'error') {
            videoPlaybackActive = false;
            window._isPictureInPicture = false;
        }
        if (signal === 'playing' || signal === 'finished'
            || signal === 'canceled' || signal === 'error') {
            updatePictureInPictureUi();
        }
        if (window.api && window.api.player && window.api.player[signal]) {
            console.debug('[Media] Firing signal:', signal);
            window.api.player[signal](...args);
        } else {
            console.error('[Media] Signal not found:', signal, 'api exists:', !!window.api);
        }
    };
    window._nativeFullscreenChanged = function(fullscreen) {
        window._isFullscreen = fullscreen;
        const player = window._mpvVideoPlayerInstance;
        if (player && player.events) {
            player.events.trigger(player, 'fullscreenchange');
        }
    };
    function isPictureInPictureControl(node) {
        if (!(node instanceof Element)) return false;
        const button = node.matches('button') ? node : node.closest('button');
        if (!button || button.closest('jmp-pip-close')) return false;
        const descendants = [...button.querySelectorAll('*')];
        const identity = [button, ...descendants]
            .flatMap(element => [element.className, element.id, element.title,
                element.getAttribute('aria-label')])
            .map(value => String(value || '').toLowerCase())
            .join(' ');
        if (identity.includes('pictureinpicture') || identity.includes('picture-in-picture')
            || identity.includes('picture_in_picture') || identity.includes('btnpip')) {
            return true;
        }
        const icon = button.querySelector('.material-icons, .material-symbols-rounded, .material-symbols-outlined');
        return icon && ['picture_in_picture', 'picture_in_picture_alt']
            .includes(icon.textContent.trim().toLowerCase());
    }

    function pictureInPictureControlKind(button) {
        const identity = [button, ...button.querySelectorAll('*')]
            .flatMap(element => [element.className, element.id, element.title,
                element.getAttribute('aria-label'), element.textContent])
            .map(value => String(value || '').toLowerCase())
            .join(' ');
        if (/btnprevious|previoustrack|skip_previous|previous track/.test(identity)) return 'previous';
        if (/btnnext|nexttrack|skip_next|next track/.test(identity)) return 'next';
        if (/btnpause|btnplay|play_pause|play pause|\bpause\b|\bplay\b|play_arrow/.test(identity)) return 'play-pause';
        if (/btnaudiotracks|audio track|audio language|audiotrack/.test(identity)) return 'audio';
        if (/btnsubtitles|subtitle|captions|closed caption/.test(identity)) return 'subtitles';
        return null;
    }

    function markPictureInPictureControlGroup(element) {
        const group = element.closest('.buttons');
        if (!group) return;
        let child = element;
        while (child.parentElement && child.parentElement !== group) {
            child = child.parentElement;
        }
        child.dataset.jmpPipControlGroup = '1';
    }

    function markPictureInPicturePlayerControls(root) {
        if (!(root instanceof Element) && root !== document) return;
        const osd = root.matches?.('.videoOsdBottom')
            ? root : root.querySelector?.('.videoOsdBottom') || document.querySelector('.videoOsdBottom');
        if (!osd) return;

        for (const button of osd.querySelectorAll('button')) {
            const kind = pictureInPictureControlKind(button);
            if (kind) {
                button.dataset.jmpPipControl = kind;
                markPictureInPictureControlGroup(button);
            }
        }
        for (const volume of osd.querySelectorAll('.volumeButtons, .osdVolumeSliderContainer')) {
            volume.dataset.jmpPipVolume = '1';
            markPictureInPictureControlGroup(volume);
            for (const button of volume.querySelectorAll('button')) {
                button.dataset.jmpPipControl = 'volume';
                markPictureInPictureControlGroup(button);
            }
        }
    }

    function markPictureInPictureSelectionMenus() {
        const selectors = '[role="dialog"], [role="menu"], [role="listbox"], ' +
            '.dialogContainer, .selectMenu, .selectMenuPage, .selectMenuContent, ' +
            '.selectMenuContainer, .selectDialog, #selectMenuPage, .actionSheet';
        const menus = [...document.querySelectorAll(selectors)].filter(menu => {
            const style = getComputedStyle(menu);
            const bounds = menu.getBoundingClientRect();
            return !menu.hidden && menu.getAttribute('aria-hidden') !== 'true'
                && style.display !== 'none' && Number(style.opacity) > 0
                && bounds.width > 0 && bounds.height > 0;
        });
        for (const menu of document.querySelectorAll('[data-jmp-pip-menu="1"]')) {
            if (!menus.includes(menu)) menu.removeAttribute('data-jmp-pip-menu');
        }
        if (!window._isPictureInPicture || !pictureInPictureMenuRequested) return;
        const selectionMenus = menus.filter(menu => !menu.closest('.videoOsdBottom'));
        for (const menu of selectionMenus) {
            if (menu.dataset.jmpPipMenu !== '1') menu.dataset.jmpPipMenu = '1';
        }
        if (selectionMenus.length > 0) pictureInPictureMenuOpened = true;
        if (selectionMenus.length === 0 && pictureInPictureMenuOpened) {
            pictureInPictureMenuRequested = false;
            pictureInPictureMenuOpened = false;
            document.documentElement.classList.remove('jmp-pip-menu-requested');
        }
    }

    function closePictureInPictureSelectionMenus() {
        const selectors = '[role="dialog"], [role="menu"], [role="listbox"], ' +
            '.dialogContainer, .selectMenu, .selectMenuPage, .selectMenuContent, ' +
            '.selectMenuContainer, .selectDialog, #selectMenuPage, .actionSheet';
        const menus = [...document.querySelectorAll(selectors)].filter(menu => {
            if (menu.closest('.videoOsdBottom')) return false;
            const style = getComputedStyle(menu);
            const bounds = menu.getBoundingClientRect();
            return !menu.hidden && menu.getAttribute('aria-hidden') !== 'true'
                && style.display !== 'none' && Number(style.opacity) > 0
                && bounds.width > 0 && bounds.height > 0;
        });
        if (menus.length === 0) return false;

        for (const menu of menus.filter(candidate => !candidate.parentElement?.closest(selectors))) {
            if (menu instanceof HTMLDialogElement && menu.open) {
                menu.close();
                continue;
            }
            const closeButton = menu.querySelector(
                '.btnClose, .btnCancel, .closeButton, button[aria-label="Close"], button[title="Close"]'
            );
            if (closeButton instanceof HTMLElement) closeButton.click();
        }
        document.dispatchEvent(new KeyboardEvent('keydown', {
            key: 'Escape',
            code: 'Escape',
            bubbles: true,
            cancelable: true
        }));
        document.documentElement.classList.remove('jmp-pip-menu-requested');
        pictureInPictureMenuRequested = false;
        pictureInPictureMenuOpened = false;
        requestAnimationFrame(markPictureInPictureSelectionMenus);
        return true;
    }

    function markPictureInPictureControls(root) {
        if (!(root instanceof Element) && root !== document) return;
        const candidates = [];
        if (root instanceof Element && root.matches('button')) candidates.push(root);
        if (root.querySelectorAll) candidates.push(...root.querySelectorAll('button'));
        for (const candidate of candidates) {
            if (isPictureInPictureControl(candidate)) {
                candidate.dataset.jmpPictureInPictureControl = '1';
            }
        }
    }

    function updatePictureInPictureUi() {
        document.documentElement.classList.toggle('jmp-pip-active', window._isPictureInPicture);
        if (window._isPictureInPicture) {
            markPictureInPictureControls(document);
            markPictureInPicturePlayerControls(document);
            markPictureInPictureSelectionMenus();
        } else {
            pictureInPictureMenuRequested = false;
            pictureInPictureMenuOpened = false;
            document.documentElement.classList.remove('jmp-pip-menu-requested');
        }
        if (pictureInPictureButton) {
            pictureInPictureHost.dataset.visible =
                videoPlaybackActive && videoOsdVisible ? '1' : '0';
            pictureInPictureButton.title = window._isPictureInPicture
                ? 'Exit picture-in-picture' : 'Picture-in-picture';
            pictureInPictureButton.setAttribute('aria-label', pictureInPictureButton.title);
        }
        if (pictureInPictureCloseHost) {
            pictureInPictureCloseHost.dataset.visible = window._isPictureInPicture ? '1' : '0';
        }
    }

    function makePictureInPictureIcon() {
        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svg.setAttribute('viewBox', '0 0 24 24');
        svg.setAttribute('aria-hidden', 'true');
        svg.innerHTML = '<path fill="currentColor" d="M19 11h-8v6h8v-6zm2 8V5H3v14h18zM5 7h14v10H5V7z"/>';
        return svg;
    }

    function buildPictureInPictureButton() {
        if (pictureInPictureButton || !window.jmpNative?.setPictureInPicture) return;
        const controls = document.querySelector('.videoOsdBottom .buttons, .videoOsdBottom');
        if (!controls) return;
        const serverButton = [...document.querySelectorAll('button')].find(isPictureInPictureControl);
        if (serverButton) {
            pictureInPictureButton = serverButton;
            pictureInPictureHost = serverButton;
            serverButton.dataset.jmpPictureInPictureControl = '1';
            updatePictureInPictureUi();
            return;
        }
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'paper-icon-button-light';
        button.title = 'Picture-in-picture';
        button.setAttribute('aria-label', button.title);
        button.dataset.jmpPictureInPictureControl = '1';
        button.addEventListener('click', event => {
            event.preventDefault();
            event.stopPropagation();
            const player = window._mpvVideoPlayerInstance;
            if (player && typeof player.setPictureInPictureEnabled === 'function') {
                player.setPictureInPictureEnabled(!window._isPictureInPicture);
            } else if (window.jmpNative?.setPictureInPicture) {
                window.jmpNative.setPictureInPicture(!window._isPictureInPicture, 1);
            }
        });
        button.style.cssText = 'color:inherit;';
        pictureInPictureHost = button;
        button.replaceChildren(makePictureInPictureIcon());
        controls.appendChild(button);
        pictureInPictureButton = button;
        updatePictureInPictureUi();
    }

    function buildPictureInPictureCloseButton() {
        if (pictureInPictureCloseHost) return;
        const host = document.createElement('jmp-pip-close');
        const root = host.attachShadow({ mode: 'closed' });
        const style = document.createElement('style');
        style.textContent = `
            :host { position: fixed; inset: 0; z-index: 2147483647;
                    display: none; pointer-events: none; }
            :host([data-visible="1"]) { display: block; }
            .drag { position: absolute; z-index: 0; top: 0; left: 0;
                    right: calc(4vmin + clamp(22px, 10vmin, 38px));
                    height: calc(4vmin + clamp(22px, 10vmin, 38px));
                    pointer-events: auto; cursor: move; }
            button { all: unset; position: absolute; z-index: 1; top: 2vmin; right: 2vmin;
                     box-sizing: border-box; width: clamp(22px, 10vmin, 38px);
                     height: clamp(22px, 10vmin, 38px);
                     border-radius: 50%; background: rgba(20, 20, 20, .72); color: white;
                     cursor: pointer; pointer-events: auto; box-shadow: 0 1px 5px rgba(0, 0, 0, .45); }
            button:hover, button:focus-visible { background: #c42b1c; }
            button::before, button::after { content: ''; position: absolute; top: 50%; left: 50%;
                                            width: 48%; height: 2px; border-radius: 1px;
                                            background: currentColor; }
            button::before { transform: translate(-50%, -50%) rotate(45deg); }
            button::after { transform: translate(-50%, -50%) rotate(-45deg); }`;
        const button = document.createElement('button');
        button.type = 'button';
        button.title = 'Exit picture-in-picture';
        button.setAttribute('aria-label', 'Exit picture-in-picture');
        button.addEventListener('mousedown', event => event.stopPropagation());
        button.addEventListener('click', event => {
            event.preventDefault();
            event.stopPropagation();
            const player = window._mpvVideoPlayerInstance;
            if (player && typeof player.setPictureInPictureEnabled === 'function') {
                player.setPictureInPictureEnabled(false);
            } else if (window.jmpNative?.setPictureInPicture) {
                window.jmpNative.setPictureInPicture(false, 1);
            }
        });
        const drag = document.createElement('div');
        drag.className = 'drag';
        drag.addEventListener('mousedown', event => {
            if (event.button !== 0) return;
            event.preventDefault();
            event.stopPropagation();
            window.jmpNative?.windowStartMove?.();
        });
        root.append(style, drag, button);
        pictureInPictureCloseHost = host;
        document.documentElement.appendChild(host);
        updatePictureInPictureUi();
    }

    window._isPictureInPicture = false;
    window._nativePictureInPictureChanged = function(active) {
        window._isPictureInPicture = !!active;
        updatePictureInPictureUi();
        const player = window._mpvVideoPlayerInstance;
        if (player && player.events) {
            player.events.trigger(player, 'pictureinpicturechange');
        }
    };
    window._nativeUpdatePosition = function(ms) {
        playerState.position = ms;
        window.api.player.positionUpdate(ms);
    };
    window._nativeUpdateDuration = function(ms) {
        playerState.duration = ms;
        window.api.player.updateDuration(ms);
    };
    // Native emitters for media session control commands
    window._nativeHostInput = function(actions) {
        console.debug('[Media] _nativeHostInput:', actions);
        window.api.input.hostInput(actions);
    };
    window._nativeSetRate = function(rate) {
        console.debug('[Media] _nativeSetRate:', rate);
        window.api.input.rateChanged(rate);
    };
    window._nativeSeek = function(positionMs) {
        console.debug('[Media] _nativeSeek:', positionMs);
        window.api.input.positionSeek(positionMs);
    };

    // window.NativeShell - app info and plugins
    const plugins = ['mpvVideoPlayer', 'mpvAudioPlayer', 'inputPlugin'];
    for (const plugin of plugins) {
        window[plugin] = () => window['_' + plugin];
    }

    window.NativeShell = {
        openUrl(url, target) {
            window.api.system.openExternalUrl(url);
        },
        downloadFile(info) {
            window.api.system.openExternalUrl(info.url);
        },
        openClientSettings() {
            // The settings surface is native. If an older/server-side shim
            // left its legacy page mounted, remove that route before opening
            // the native surface so the two settings UIs cannot stack.
            window._nativeClientSettingsClosed?.();
            window.jmpNative.openClientSettings();
        },
        openAbout() {
            window.jmpNative.openAbout();
        },
        getPlugins() {
            return plugins;
        }
    };

    // The native settings modal owns the visible settings UI. Older web
    // integrations can still leave clientSettingsPage in the Jellyfin page
    // stack; the native close path calls this to restore the player route.
    window._nativeClientSettingsClosed = function() {
        const page = document.getElementById('clientSettingsPage');
        if (page) window.history.back();
    };

    // Device profile for direct play. Built in C++ at startup from mpv's
    // actual decoder/demuxer/protocol support and injected here as a JSON
    // literal (JSON is a subset of JS object syntax, so no parse needed).
    const _deviceProfile = __DEVICE_PROFILE_JSON__;
    function getDeviceProfile() {
        return _deviceProfile;
    }

    window.NativeShell.AppHost = {
        init() {
            return Promise.resolve({
                deviceName: jmpInfo.deviceName,
                appName: 'Jellium Desktop - EAJelly',
                appVersion: jmpInfo.version
            });
        },
        getDefaultLayout() {
            return jmpInfo.mode;
        },
        supports(command) {
            const features = [
                'fileinput', 'filedownload', 'displaylanguage', 'htmlaudioautoplay',
                'htmlvideoautoplay', 'externallinks',
                'fullscreenchange', 'remotevideo', 'displaymode',
                'exitmenu', 'clientsettings'
            ];
            return features.includes(command.toLowerCase());
        },
        getDeviceProfile,
        getSyncProfile: getDeviceProfile,
        appName() { return 'Jellium Desktop - EAJelly'; },
        appVersion() { return jmpInfo.version; },
        deviceName() { return jmpInfo.deviceName; },
        exit() { window.api.system.exit(); }
    };

    window.initCompleted = Promise.resolve();
    window.apiPromise = Promise.resolve(window.api);

    // Observe <meta name="theme-color"> for titlebar color sync.
    // jellyfin-web's themeManager.js updates this tag when the user switches themes.
    function sendThemeColor(color) {
        if (color && window.jmpNative && window.jmpNative.themeColor) {
            window.jmpNative.themeColor(color);
        }
    }

    function observeThemeColorMeta(meta) {
        sendThemeColor(meta.content);
        new MutationObserver(() => sendThemeColor(meta.content))
            .observe(meta, { attributes: true, attributeFilter: ['content'] });
    }

    document.addEventListener('DOMContentLoaded', () => {
        // Inject CSS to hide cursor when jellyfin-web signals mouse idle.
        // jellyfin-web adds 'mouseIdle' to body after inactivity during video playback.
        // This CSS makes CEF report CT_NONE so the native side can hide the OS cursor.
        const style = document.createElement('style');
        let css = 'body.mouseIdle, body.mouseIdle * { cursor: none !important; }';
        css += '\n@keyframes mpv-video-zoomin { from { transform: scale3d(0.2, 0.2, 0.2); opacity: 0.6; } to { transform: none; opacity: initial; } }';
        css += '\nhtml.jmp-pip-active [data-jmp-picture-in-picture-control="1"] { display: none !important; }';
        css += '\nhtml.jmp-pip-active body { visibility: hidden !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom { display: block !important; position: fixed !important; left: 0 !important; right: 0 !important; bottom: 0 !important; top: auto !important; width: 100% !important; max-width: none !important; min-height: 0 !important; margin: 0 !important; padding: 12px 10px 10px !important; box-sizing: border-box !important; transform: none !important; opacity: 1 !important; visibility: visible !important; z-index: 2147483644 !important; background: linear-gradient(transparent, rgba(0,0,0,.72) 30%, rgba(0,0,0,.88)) !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom.videoOsdBottom-hidden { opacity: 1 !important; transform: none !important; visibility: visible !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom * { visibility: hidden !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .osdControls > .flex, html.jmp-pip-active .videoOsdBottom .osdTextContainer, html.jmp-pip-active .videoOsdBottom .osdPositionText, html.jmp-pip-active .videoOsdBottom .osdDurationText, html.jmp-pip-active .videoOsdBottom .endsAtText, html.jmp-pip-active .videoOsdBottom .osdMainTextContainer { display: none !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons { display: flex !important; visibility: visible !important; align-items: center !important; justify-content: center !important; gap: clamp(2px, 1vw, 10px) !important; width: max-content !important; max-width: 100% !important; margin: 0 auto !important; padding: 4px 10px !important; border: 1px solid rgba(255,255,255,.14) !important; border-radius: 999px !important; background: rgba(18,20,24,.72) !important; box-shadow: 0 4px 18px rgba(0,0,0,.34) !important; backdrop-filter: blur(12px) !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons > *:not([data-jmp-pip-control-group="1"]) { display: none !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons button:not([data-jmp-pip-control]) { display: none !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons, html.jmp-pip-active .videoOsdBottom .buttons * { visibility: hidden !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons [data-jmp-pip-control-group="1"], html.jmp-pip-active .videoOsdBottom .buttons [data-jmp-pip-control], html.jmp-pip-active .videoOsdBottom .buttons [data-jmp-pip-control] *, html.jmp-pip-active .videoOsdBottom .buttons [data-jmp-pip-volume], html.jmp-pip-active .videoOsdBottom .buttons [data-jmp-pip-volume] * { display: flex !important; visibility: visible !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons .paper-icon-button-light { display: inline-flex !important; align-items: center !important; justify-content: center !important; flex: 0 0 auto !important; width: clamp(28px, 7vmin, 42px) !important; height: clamp(28px, 7vmin, 42px) !important; min-width: clamp(28px, 7vmin, 42px) !important; min-height: clamp(28px, 7vmin, 42px) !important; margin: 0 !important; padding: 0 !important; border: 0 !important; border-radius: 50% !important; background: transparent !important; color: rgba(255,255,255,.94) !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons .paper-icon-button-light:hover { background: rgba(255,255,255,.14) !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons .paper-icon-button-light .material-icons { font-size: clamp(18px, 4.2vmin, 24px) !important; line-height: 1 !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons .btnPause.paper-icon-button-light { width: clamp(32px, 8vmin, 48px) !important; height: clamp(32px, 8vmin, 48px) !important; min-width: clamp(32px, 8vmin, 48px) !important; min-height: clamp(32px, 8vmin, 48px) !important; background: rgba(255,255,255,.16) !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons .volumeButtons[data-jmp-pip-volume] { display: flex !important; align-items: center !important; flex: 0 0 auto !important; gap: 2px !important; margin: 0 !important; padding: 0 !important; background: none !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons .osdVolumeSliderContainer[data-jmp-pip-volume] { display: flex !important; align-items: center !important; flex: 0 1 clamp(56px, 12vw, 112px) !important; width: clamp(56px, 12vw, 112px) !important; min-width: 48px !important; max-width: 112px !important; margin: 0 4px !important; opacity: 1 !important; visibility: visible !important; }';
        css += '\nhtml.jmp-pip-active .videoOsdBottom .buttons .osdVolumeSliderContainer[data-jmp-pip-volume] .mdl-slider-container, html.jmp-pip-active .videoOsdBottom .buttons .osdVolumeSliderContainer[data-jmp-pip-volume] input[type="range"] { display: block !important; width: 100% !important; opacity: 1 !important; visibility: visible !important; }';
        css += '\nhtml.jmp-pip-active [data-jmp-pip-menu="1"], html.jmp-pip-active [data-jmp-pip-menu="1"] * { visibility: visible !important; }';

        // Hide scrollbars app-wide (scroll still works via wheel/trackpad/keys).
        if (jmpInfo.settings.advanced.hideScrollbar) {
            css += '\n::-webkit-scrollbar, *::-webkit-scrollbar { width: 0 !important; height: 0 !important; display: none !important; }';
            css += '\nhtml { scrollbar-width: none !important; }';
        }

        // macOS: offset UI elements so traffic lights don't overlap content
        if (navigator.platform.startsWith('Mac') && jmpInfo.settings.advanced.transparentTitlebar) {
            css += '\n:root { --mac-titlebar-height: 22px; }';
            css += '\n.skinHeader { padding-top: var(--mac-titlebar-height) !important; }';
            css += '\n.mainAnimatedPage { top: var(--mac-titlebar-height) !important; }';
            css += '\n.touch-menu-la { padding-top: var(--mac-titlebar-height); }';
            // Dashboard uses MUI AppBar + Drawer instead of .skinHeader
            css += '\n.MuiAppBar-positionFixed { padding-top: var(--mac-titlebar-height) !important; }';
            css += '\n.MuiDrawer-paper { padding-top: var(--mac-titlebar-height) !important; }';
            // Dialog headers (e.g. client settings modal)
            css += '\n.formDialogHeader { padding-top: var(--mac-titlebar-height) !important; }';

        }

        style.textContent = css;
        document.head.appendChild(style);

        buildPictureInPictureButton();
        buildPictureInPictureCloseButton();
        markPictureInPictureControls(document);
        markPictureInPicturePlayerControls(document);
        new MutationObserver((mutations) => {
            for (const mutation of mutations) {
                for (const node of mutation.addedNodes) {
                    if (node instanceof Element) {
                        markPictureInPictureControls(node);
                        markPictureInPicturePlayerControls(node);
                        if (!pictureInPictureButton) buildPictureInPictureButton();
                    }
                }
            }
            if (window._isPictureInPicture && pictureInPictureMenuRequested) {
                markPictureInPictureSelectionMenus();
            }
        }).observe(document.body, {
            childList: true,
            subtree: true,
            attributes: true,
            attributeFilter: ['class', 'style', 'aria-hidden', 'hidden']
        });

        document.addEventListener('click', event => {
            if (!window._isPictureInPicture || !(event.target instanceof Element)) return;
            const control = event.target.closest('.videoOsdBottom [data-jmp-pip-control]');
            if (!control) return;
            const kind = control.dataset.jmpPipControl;
            if (closePictureInPictureSelectionMenus() && (kind === 'audio' || kind === 'subtitles')) {
                event.preventDefault();
                event.stopImmediatePropagation();
                return;
            }
            if (kind !== 'audio' && kind !== 'subtitles') return;
            const selectionControl = event.target.closest(
                '[data-jmp-pip-control="audio"], [data-jmp-pip-control="subtitles"]'
            );
            if (!selectionControl) return;
            pictureInPictureMenuRequested = true;
            pictureInPictureMenuOpened = false;
            document.documentElement.classList.add('jmp-pip-menu-requested');
            requestAnimationFrame(markPictureInPictureSelectionMenus);
        }, true);

        // The PiP close overlay follows the same visibility signal as the
        // player controls. macOS traffic lights continue to consume it too.
        document._callbacks = document._callbacks || {};
        document._callbacks['SHOW_VIDEO_OSD'] = document._callbacks['SHOW_VIDEO_OSD'] || [];
        document._callbacks['SHOW_VIDEO_OSD'].push((_e, visible) => {
            videoOsdVisible = !!visible;
            updatePictureInPictureUi();
            if (navigator.platform.startsWith('Mac')
                && jmpInfo.settings.advanced.transparentTitlebar
                && window.jmpNative?.setOsdVisible) {
                window.jmpNative.setOsdVisible(videoOsdVisible);
            }
        });

        // Sync titlebar color with theme-color meta tag
        const meta = document.querySelector('meta[name="theme-color"]');
        if (meta) {
            observeThemeColorMeta(meta);
        } else {
            // Tag may be added dynamically — watch for it
            new MutationObserver((mutations, obs) => {
                for (const m of mutations) {
                    for (const node of m.addedNodes) {
                        if (node.nodeName === 'META' && node.name === 'theme-color') {
                            obs.disconnect();
                            observeThemeColorMeta(node);
                            return;
                        }
                    }
                }
            }).observe(document.head, { childList: true });
        }

        // Check once per launch. On Windows, accepting the prompt downloads
        // the architecture-matched Inno installer in native code, launches it,
        // and then cleanly closes this process. Other platforms retain the
        // manual updater in About until a native package handoff is available.
        setTimeout(() => {
            const assetSuffix = '__UPDATE_ASSET_SUFFIX__';
            if (!assetSuffix || document.getElementById('_jelliumUpdate')) return;

            const versionParts = value => String(value || '')
                .trim()
                .replace(/^v/i, '')
                .split(/[+-]/, 1)[0]
                .split('.')
                .map(part => parseInt(part, 10) || 0);
            const isNewer = (candidate, current) => {
                const left = versionParts(candidate);
                const right = versionParts(current);
                for (let i = 0; i < Math.max(left.length, right.length, 3); i++) {
                    const difference = (left[i] || 0) - (right[i] || 0);
                    if (difference) return difference > 0;
                }
                return false;
            };

            fetch('https://api.github.com/repos/eaforlife/jellium-desktop-eajelly/releases/latest', {
                cache: 'no-store'
            })
                .then(response => {
                    if (!response.ok) throw new Error('HTTP ' + response.status);
                    return response.json();
                })
                .then(release => {
                    const latest = release.tag_name || release.name;
                    if (!latest || !isNewer(latest, jmpInfo.version)) return;
                    const asset = Array.isArray(release.assets)
                        ? release.assets.find(item => String(item.name || '').endsWith(assetSuffix))
                        : null;
                    if (!asset || !asset.browser_download_url || !asset.digest || !asset.size) return;

                    const host = document.createElement('div');
                    host.id = '_jelliumUpdate';
                    host.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:rgba(0,0,0,.55);display:flex;align-items:center;justify-content:center';
                    const box = document.createElement('div');
                    box.style.cssText = 'width:min(440px,calc(100vw - 40px));padding:22px;border-radius:8px;background:#202020;color:#eee;font:14px/1.45 sans-serif;box-shadow:0 8px 32px #000';
                    const title = document.createElement('h2');
                    title.textContent = 'Jellium Desktop - EAJelly update';
                    title.style.cssText = 'margin:0 0 10px;font-size:20px';
                    const message = document.createElement('p');
                    message.textContent = `${latest} is available. Download it now? The app will close and the installer will open when the download finishes.`;
                    message.style.cssText = 'margin:0 0 18px;color:#ccc';
                    const actions = document.createElement('div');
                    actions.style.cssText = 'display:flex;justify-content:flex-end;gap:10px';
                    const later = document.createElement('button');
                    later.textContent = 'Later';
                    const install = document.createElement('button');
                    install.textContent = 'Download and install';
                    for (const button of [later, install]) {
                        button.style.cssText = 'border:0;border-radius:4px;padding:8px 12px;cursor:pointer';
                    }
                    install.style.background = '#00a4dc';
                    install.style.color = '#fff';
                    later.addEventListener('click', () => host.remove());
                    install.addEventListener('click', () => {
                        if (!window.jmpNative || !window.jmpNative.installUpdate) return;
                        later.disabled = true;
                        install.disabled = true;
                        install.textContent = 'Downloading…';
                        message.textContent = 'Downloading the update. Jellium Desktop - EAJelly will close automatically when the installer is ready.';
                        window.jmpNative.installUpdate(asset.browser_download_url, asset.digest, asset.size);
                    });
                    actions.append(later, install);
                    box.append(title, message, actions);
                    host.appendChild(box);
                    document.body.appendChild(host);
                })
                .catch(error => console.warn('[Updater] Launch check failed:', error));
        }, 5000);
    });

    console.debug('[Media] Native shim installed');
})();
