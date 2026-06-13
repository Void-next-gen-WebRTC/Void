import { createContext, ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { ClientSignalMessage } from '../types/clientSignal.type';
import VoicePeer from '../models/voice/voicePeer.model';
import ChatMessage from '../models/chat/chatMessage.model';
import ExtendedVoiceState from '../models/voice/extendedVoiceState.model';
import initWasm from '../pkg/core_wasm';
import { invoke } from '@tauri-apps/api/core';
import WebSocket from '@tauri-apps/plugin-websocket';
import { useAuth } from './AuthContext';
import { useToast } from './ToastContext';
import { useNetworkStats } from '../hooks/useNetworkStats';
import { usePushToTalk } from '../hooks/usePushToTalk';
import { useFingerprint } from '../hooks/useFingerprint';
import { useVoiceSettings } from '../hooks/useVoiceSettings';
import { useSfuConnection } from '../hooks/useSfuConnection';
import { useChannelManager } from '../hooks/useChannelManager';
import { setSignalingSender } from '../lib/signalingTransport';
import { getToken } from '../api/http-client';
import { config } from '../lib/config';

// Use the centralized signaling URL resolution from `lib/config` so the
// production fallback (Oracle VM) is honoured when `VITE_SIGNALING_URL`
// is not injected at build time. The previous hard-coded `127.0.0.1`
// fallback caused each prod client to dial its own loopback, which
// silently isolated users into per-machine "phantom" rooms.
const SIGNALING_URL = config.wsUrl.replace(/^["']/, "").replace(/["']$/, "").trim();

const VoiceContext = createContext<ExtendedVoiceState | undefined>(undefined);

export const VoiceProvider = ({ children }: { children: ReactNode }) => {
    const [channelId, setChannelId] = useState<string | null>(null);
    const [participants, setParticipants] = useState<VoicePeer[]>([]);
    const [channelStartedAt, setChannelStartedAt] = useState<number | undefined>(undefined);
    const [isConnected, setIsConnected] = useState(false);
    const [isMuted, setIsMuted] = useState(false);
    const [isDeafened, setIsDeafened] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [remoteStreams, setRemoteStreams] = useState<Map<string, MediaStream>>(new Map());
    const [remoteVideoStreams, setRemoteVideoStreams] = useState<Map<string, MediaStream>>(new Map());
    const [localStream, setLocalStream] = useState<MediaStream | null>(null);
    const [rawLocalStream, setRawLocalStream] = useState<MediaStream | null>(null);
    const [userVolumes, setUserVolumes] = useState<Map<string, number>>(new Map());
    const [chatMessages, setChatMessages] = useState<ChatMessage[]>(() => {
        try {
            const _stored = localStorage.getItem('void_chat_messages');
            return _stored ? JSON.parse(_stored) : [];
        } catch { return []; }
    });
    const [bandwidthStats, setBandwidthStats] = useState<Map<string, number>>(new Map());
    const [wasmReady, setWasmReady] = useState(false);
    const [localUserId, setLocalUserId] = useState<string>("");
    const [localUsername, setLocalUsername] = useState<string>("");

    const { addToast } = useToast();
    const { username: authUsername } = useAuth();
    const fingerprintRef = useFingerprint(wasmReady);

    const socketRef = useRef<WebSocket | null>(null);
    const localStreamRef = useRef<MediaStream | null>(null);
    const localAudioCtxRef = useRef<AudioContext | null>(null);
    const noiseGateNodeRef = useRef<AudioWorkletNode | null>(null);
    const userIdRef = useRef<string>("");
    const usernameRef = useRef<string>("");
    const channelIdRef = useRef<string | null>(null);
    const signalQueueRef = useRef<ClientSignalMessage[]>([]);
    const reconnectTimerRef = useRef<number | null>(null);
    const connectSocketRef = useRef<(() => Promise<void>) | null>(null);

    const scheduleReconnect = useCallback((delayMs: number) => {
        if (reconnectTimerRef.current !== null) {
            return;
        }
        reconnectTimerRef.current = window.setTimeout(() => {
            reconnectTimerRef.current = null;
            void connectSocketRef.current?.();
        }, delayMs);
    }, []);

    // ── Extracted hooks ──────────────────────────────────────────────
    const settings = useVoiceSettings({ noiseGateNodeRef, username: authUsername });

    const { isPttActive, enforceTrackEnabled } = usePushToTalk({
        vadMode: settings.vadMode, pttKey: settings.pttKey,
        isMuted, localStreamRef,
    });

    const sendSignal = useCallback(async (payload: ClientSignalMessage) => {
        if (socketRef.current) {
            try {
                console.log('[VOICE] sendSignal →', payload);
                await socketRef.current.send(JSON.stringify(payload));
            }
            catch (err) { console.error("[VOICE] Signal send error:", err); }
        } else {
            console.log('[VOICE] sendSignal QUEUED (no socket yet) →', payload);
            signalQueueRef.current.push(payload);
        }
    }, []);

    const { sfuConnectionRef, screenStreamRef, handleMessage, addScreenTrack, removeScreenTrack } =
        useSfuConnection({
            sendSignal, localStreamRef, userIdRef, usernameRef, addToast,
            setParticipants, setChannelStartedAt, setRemoteStreams,
            setRemoteVideoStreams, setChatMessages, setBandwidthStats, setError,
        });

    const { joinChannel, leaveChannel } = useChannelManager({
        sendSignal, sfuConnectionRef, localStreamRef, localAudioCtxRef,
        screenStreamRef, noiseGateNodeRef, channelIdRef, userIdRef,
        usernameRef, fingerprintRef, rawMicVolumeRef: settings.rawMicVolumeRef,
        remoteStreams, smartGateEnabled: settings.smartGateEnabled,
        vadThreshold: settings.vadThreshold, vadAuto: settings.vadAuto,
        setLocalStream, setRawLocalStream, setChannelId, setParticipants,
        setChannelStartedAt, setRemoteStreams, setRemoteVideoStreams, setError,
    });

    // ── Refs sync ────────────────────────────────────────────────────
    useEffect(() => { userIdRef.current = localUserId; }, [localUserId]);
    useEffect(() => { usernameRef.current = localUsername; }, [localUsername]);

    // ── Socket connection ────────────────────────────────────────────
    const connectSocket = useCallback(async () => {
        console.log('[VOICE] connectSocket called', {
            userId: userIdRef.current,
            hasSocket: !!socketRef.current,
            url: SIGNALING_URL,
        });
        if (!userIdRef.current || socketRef.current) {
            console.log('[VOICE] connectSocket SKIP (no userId or already connected)');
            return;
        }

        const _checkUrl = SIGNALING_URL
            .replace(/^wss?:\/\//, (m: string) => m === 'wss://' ? 'https://' : 'http://')
            .replace(/\/ws$/, '/health');
        try {
            console.log('[VOICE] health check →', _checkUrl);
            await invoke('call_signaling', { url: _checkUrl });
            console.log('[VOICE] health check OK');
        } catch (err) {
            // Health pre-check is informational only: WS handshake decides the real state.
            console.warn('[VOICE] health check failed, continuing with WS connect:', err);
        }

        let socket: WebSocket;
        try {
            console.log('[VOICE] WebSocket.connect →', SIGNALING_URL);
            socket = await WebSocket.connect(SIGNALING_URL);
        } catch (err) {
            console.error('[VOICE] WS connect failed:', err);
            setIsConnected(false);
            socketRef.current = null;
            scheduleReconnect(5_000);
            return;
        }

        try {
            socketRef.current = socket;
            setIsConnected(true);
            console.log('[VOICE] WebSocket OPEN');

            // Expose the active sender to feature modules (RPC, subscriptions).
            setSignalingSender((payload) => socket.send(JSON.stringify(payload)));

            await socket.addListener((msg) => {
                if (msg.type === 'Text') {
                    console.log('[VOICE] WS RX', (msg.data as string).slice(0, 200));
                    handleMessage(msg.data);
                }
                if (msg.type === 'Close') {
                    console.warn('[VOICE] WS CLOSED, will reconnect in 3s');
                    socketRef.current = null;
                    setSignalingSender(null);
                    setIsConnected(false);
                    scheduleReconnect(3_000);
                }
            });

            // Authenticate the WS so RPC calls can use the JWT identity.
            const _token = getToken();
            console.log('[VOICE] authenticate →', _token ? `token(${_token.length}b)` : 'NO TOKEN');
            if (_token) {
                await socket.send(JSON.stringify({ type: 'authenticate', token: _token }));
            }

            const _initialJoin: ClientSignalMessage = {
                type: 'join',
                channelId: channelIdRef.current || 'global',
                userId: userIdRef.current,
                username: usernameRef.current,
                ...(fingerprintRef.current ? { fingerprint: fingerprintRef.current } : {}),
            };
            console.log('[VOICE] initial join →', _initialJoin);
            await socket.send(JSON.stringify(_initialJoin));

            console.log('[VOICE] flushing queue:', signalQueueRef.current.length, 'pending');
            while (signalQueueRef.current.length > 0) {
                const _signal = signalQueueRef.current.shift();
                if (_signal) {
                    console.log('[VOICE] flush →', _signal);
                    await socket.send(JSON.stringify(_signal));
                }
            }
        } catch (err) {
            console.error('[VOICE] WS setup failed:', err);
            setIsConnected(false);
            socketRef.current = null;
            setSignalingSender(null);
            try {
                await socket.disconnect();
            } catch {
                // Ignore disconnect errors on already broken sockets.
            }
            scheduleReconnect(5_000);
        }
    }, [fingerprintRef, handleMessage, scheduleReconnect]);

    useEffect(() => {
        connectSocketRef.current = connectSocket;
    }, [connectSocket]);

    const setUserInfo = useCallback((username: string, userId: string) => {
        console.log('[VOICE] setUserInfo', { username, userId });
        if (!username || !userId) {
            console.warn('[VOICE] setUserInfo SKIP — empty username or userId');
            return;
        }
        setLocalUsername(username);
        setLocalUserId(userId);
        usernameRef.current = username;
        userIdRef.current = userId;
        connectSocket();
    }, [connectSocket]);

    // ── Media toggles ────────────────────────────────────────────────
    const toggleMute = useCallback(() => {
        const next = !isMuted;
        setIsMuted(next);
        enforceTrackEnabled();
        setParticipants(p => p.map(part => part.userId === userIdRef.current ? { ...part, isMuted: next } : part));
        if (channelIdRef.current) {
            sendSignal({ type: 'media-state', channelId: channelIdRef.current, userId: userIdRef.current, isMuted: next, isDeafened });
        }
    }, [isMuted, isDeafened, sendSignal, enforceTrackEnabled]);

    const toggleDeafen = useCallback(() => {
        const next = !isDeafened;
        const nextMute = next || isMuted;
        setIsDeafened(next);
        enforceTrackEnabled();
        setParticipants(p => p.map(part => part.userId === userIdRef.current ? { ...part, isDeafened: next, isMuted: nextMute } : part));
        if (channelIdRef.current) {
            sendSignal({ type: 'media-state', channelId: channelIdRef.current, userId: userIdRef.current, isMuted: nextMute, isDeafened: next });
        }
    }, [isDeafened, isMuted, sendSignal, enforceTrackEnabled]);

    const sendChatMessage = useCallback((message: string, targetChannelId?: string) => {
        if (!message.trim()) return;
        const _timestamp = Date.now();
        const _id = `${userIdRef.current}-${_timestamp}`;
        const _channelId = targetChannelId || 'global-chat';

        setChatMessages(prev => [...prev, {
            id: _id,
            from: userIdRef.current,
            username: usernameRef.current,
            message: message.trim(),
            timestamp: _timestamp,
            channelId: _channelId,
        }]);

        sendSignal({
            type: 'chat', channelId: _channelId,
            from: userIdRef.current, username: usernameRef.current,
            message: message.trim(), timestamp: _timestamp,
        });
    }, [sendSignal]);

    const clearChatMessages = useCallback(() => {
        setChatMessages([]);
        localStorage.removeItem('void_chat_messages');
    }, []);

    // Persist chat messages to localStorage (capped at 200)
    useEffect(() => {
        try {
            const _limited = chatMessages.slice(-200);
            localStorage.setItem('void_chat_messages', JSON.stringify(_limited));
        } catch { /* storage full or unavailable */ }
    }, [chatMessages]);

    // ── Init & cleanup ───────────────────────────────────────────────
    useEffect(() => {
        initWasm().then(() => setWasmReady(true)).catch(() => setWasmReady(true));
        return () => {
            if (reconnectTimerRef.current !== null) {
                window.clearTimeout(reconnectTimerRef.current);
                reconnectTimerRef.current = null;
            }
            if (socketRef.current) {
                void socketRef.current.disconnect();
            }
        };
    }, []);

    const { networkQuality, ping, averagePing, packetLoss } = useNetworkStats({
        pc: sfuConnectionRef.current, isConnected, wasmReady,
    });

    // ── Context value ────────────────────────────────────────────────
    const value = useMemo(() => ({
        channelId, participants, isConnected, isMuted, isDeafened, error,
        localUserId: userIdRef.current, localStream, rawLocalStream, channelStartedAt, bandwidthStats,
        joinChannel, leaveChannel, toggleMute, toggleDeafen,
        remoteStreams, remoteVideoStreams, addScreenTrack, removeScreenTrack,
        userVolumes, setUserVolume: (id: string, vol: number) => setUserVolumes(p => new Map(p).set(id, vol)),
        networkQuality, ping, averagePing, packetLoss,
        chatMessages, sendChatMessage, clearChatMessages, setUserInfo,
        ...settings, isPttActive,
    }), [
        channelId, participants, isConnected, isMuted, isDeafened, error,
        localUserId, localStream, rawLocalStream, channelStartedAt, bandwidthStats,
        joinChannel, leaveChannel, toggleMute, toggleDeafen,
        remoteStreams, remoteVideoStreams, addScreenTrack, removeScreenTrack,
        userVolumes, networkQuality, ping, averagePing, packetLoss,
        chatMessages, sendChatMessage, clearChatMessages, setUserInfo, settings, isPttActive,
    ]);

    return <VoiceContext.Provider value={value}>{children}</VoiceContext.Provider>;
};

export const useVoiceStore = () => {
    const ctx = useContext(VoiceContext);
    if (!ctx) throw new Error("useVoiceStore must be used within VoiceProvider");
    return ctx;
};
