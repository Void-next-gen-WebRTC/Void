import { useCallback, useRef } from 'react';
import { ServerSignal } from '../types/serverSignal.type';
import UseSfuConnectionProps from '../models/voice/useSfuConnectionProps.model';
import { emitSignalingEvent } from '../lib/signalingBus';

const ICE_SERVERS: RTCIceServer[] = [
    { urls: 'stun:stun.l.google.com:19302' },
    { urls: 'stun:stun1.l.google.com:19302' },
];

/**
 * Manages the single SFU peer-connection, screen-track lifecycle,
 * and dispatching of incoming signaling messages.
 */
export function useSfuConnection({
    sendSignal, localStreamRef, userIdRef, usernameRef, addToast,
    setParticipants, setChannelStartedAt, setRemoteStreams,
    setRemoteVideoStreams, setChatMessages, setBandwidthStats, setError,
}: UseSfuConnectionProps) {

    const rememberTrackMap = (key: string | undefined | null, userId: string) => {
        if (!key) return;
        trackToUserMapRef.current.set(key, userId);
    };

    const rememberPendingTrackMap = (key: string | undefined | null, userId: string, kind: string) => {
        if (!key) return;
        pendingTrackMapsRef.current.set(key, { userId, kind });
    };

    const deletePendingTrackMap = (key: string | undefined | null) => {
        if (!key) return;
        pendingTrackMapsRef.current.delete(key);
    };

    const rememberOrphanStream = (key: string | undefined | null, stream: MediaStream, kind: string) => {
        if (!key) return;
        orphanStreamsRef.current.set(key, { stream, kind });
    };

    const deleteOrphanStream = (key: string | undefined | null) => {
        if (!key) return;
        orphanStreamsRef.current.delete(key);
    };

    const findUniquePendingByKind = (kind: string) => {
        const matches = Array.from(pendingTrackMapsRef.current.values()).filter((entry) => entry.kind === kind);
        if (matches.length !== 1) return null;
        return matches[0];
    };

    const findUniqueOrphanByKind = (kind: string) => {
        const matches = Array.from(orphanStreamsRef.current.values()).filter((entry) => entry.kind === kind);
        if (matches.length !== 1) return null;
        return matches[0];
    };

    const sfuConnectionRef = useRef<RTCPeerConnection | null>(null);
    const screenStreamRef = useRef<MediaStream | null>(null);
    const trackToUserMapRef = useRef<Map<string, string>>(new Map());
    /** Streams that arrived via `ontrack` before their `track-map` mapping. */
    const orphanStreamsRef = useRef<Map<string, { stream: MediaStream; kind: string }>>(new Map());
    /** Track-maps received before their corresponding `ontrack` event. */
    const pendingTrackMapsRef = useRef<Map<string, { userId: string; kind: string }>>(new Map());
    /**
     * TURN servers pushed by the signaling server right after auth (see
     * `case 'ice-servers'` below). `null` until the first push arrives —
     * `connectSFU` falls back to the static STUN-only `ICE_SERVERS` list
     * in that case, which is why voice/stream fails across NATs that need
     * a relay (the original cross-account bug this was added to fix).
     */
    const dynamicIceServersRef = useRef<RTCIceServer[]>([]);

    // ---- Perfect-negotiation state ----
    // The SFU is the "impolite" peer; this client is "polite". On glare
    // (offer received while a local offer is in-flight or signalingState
    // !== 'stable') the polite peer rolls back and accepts the remote offer.
    // This unblocks the audio-isolation bug observed when two users join the
    // same voice channel: the server's catch-up renegotiation offer would
    // otherwise collide with the client's initial offer and silently fail.
    const makingOfferRef = useRef<boolean>(false);
    const ignoreOfferRef = useRef<boolean>(false);
    const isSettingRemoteAnswerPendingRef = useRef<boolean>(false);

    const removeScreenTrack = useCallback(() => {
        if (screenStreamRef.current) {
            screenStreamRef.current.getTracks().forEach(t => t.stop());
            screenStreamRef.current = null;
        }
    }, []);

    const addScreenTrack = useCallback(async (stream: MediaStream) => {
        screenStreamRef.current = stream;
        const videoTrack = stream.getVideoTracks()[0];
        if (videoTrack) {
            videoTrack.onended = () => removeScreenTrack();
            if (sfuConnectionRef.current) {
                // Use addTransceiver for consistency with audio tracks.
                sfuConnectionRef.current.addTransceiver(videoTrack, { direction: 'sendonly', streams: [stream] });
            }
        }
    }, [removeScreenTrack]);

    const connectSFU = useCallback(async () => {
        console.log('[VOICE] connectSFU — creating PeerConnection');
        if (sfuConnectionRef.current) sfuConnectionRef.current.close();

        const iceServers = dynamicIceServersRef.current.length > 0
            ? [...ICE_SERVERS, ...dynamicIceServersRef.current]
            : ICE_SERVERS;
        console.log('[VOICE] connectSFU iceServers:', iceServers.map(s => s.urls));
        const pc = new RTCPeerConnection({ iceServers });
        sfuConnectionRef.current = pc;

        pc.oniceconnectionstatechange = () => console.log('[VOICE] PC ice state →', pc.iceConnectionState);
        pc.onconnectionstatechange = () => console.log('[VOICE] PC connection state →', pc.connectionState);
        pc.onsignalingstatechange = () => console.log('[VOICE] PC signaling state →', pc.signalingState);

        // Reset perfect-negotiation flags for the new PC instance.
        makingOfferRef.current = false;
        ignoreOfferRef.current = false;
        isSettingRemoteAnswerPendingRef.current = false;

        // Wait for local stream to be ready (with retries to handle race conditions)
        let retries = 50; // ~2.5s timeout (50 * 50ms)
        let local = localStreamRef.current;
        while (!local && retries > 0) {
            console.warn('[VOICE] connectSFU waiting for localStream to be ready... (' + retries + ' retries left)');
            await new Promise(resolve => setTimeout(resolve, 50));
            local = localStreamRef.current;
            retries--;
        }

        if (!local) {
            console.error('[VOICE] connectSFU FAILED: localStream not available after timeout');
            return; // Exit early if stream never appears
        }

        console.log('[VOICE] connectSFU local audio tracks:', local.getAudioTracks().length);
        // Use addTransceiver instead of addTrack for more reliable track lifecycle.
        // Explicit direction ensures tracks persist through renegotiations.
        local.getAudioTracks().forEach(t => pc.addTransceiver(t, { direction: 'sendrecv', streams: [local] }));

        const screen = screenStreamRef.current;
        if (screen) screen.getVideoTracks().forEach(t => pc.addTransceiver(t, { direction: 'sendrecv', streams: [screen] }));

        // Log track status before proceeding
        console.log('[VOICE] connectSFU tracks added:', {
            audioTrackCount: local.getAudioTracks().length,
            audioTracksEnabled: local.getAudioTracks().map(t => ({ id: t.id, enabled: t.enabled, muted: t.muted })),
            videoTrackCount: screen?.getVideoTracks().length ?? 0,
        });

        pc.onicecandidate = (e) => {
            if (e.candidate) sendSignal({ type: 'ice', candidate: e.candidate.toJSON() } as any);
        };

        pc.ontrack = (e) => {
            const track = e.track;
            // webrtc-rs often sends tracks without associated streams;
            // wrap the bare track in a fresh MediaStream so playback works.
            const stream = e.streams?.[0] ?? new MediaStream([track]);
            console.log('[VOICE] ontrack', track.kind, 'track.id=' + track.id, 'stream.id=' + stream.id, 'streams=' + e.streams.length);

            const uid = trackToUserMapRef.current.get(track.id)
                || trackToUserMapRef.current.get(stream.id);

            if (uid) {
                if (track.kind === 'audio') setRemoteStreams(r => new Map(r).set(uid, stream));
                else if (track.kind === 'video') setRemoteVideoStreams(r => new Map(r).set(uid, stream));
                return;
            }

            // No mapping yet — check pending track-maps
            const _pendingByTrack = pendingTrackMapsRef.current.get(track.id);
            const _pendingByStream = pendingTrackMapsRef.current.get(stream.id);
            const _pending = _pendingByTrack ?? _pendingByStream ?? findUniquePendingByKind(track.kind);
            if (_pending) {
                rememberTrackMap(track.id, _pending.userId);
                rememberTrackMap(stream.id, _pending.userId);
                if (track.kind === 'audio') setRemoteStreams(r => new Map(r).set(_pending.userId, stream));
                else if (track.kind === 'video') setRemoteVideoStreams(r => new Map(r).set(_pending.userId, stream));
                deletePendingTrackMap(track.id);
                deletePendingTrackMap(stream.id);
                return;
            }

            // Buffer as orphan — the track-map will resolve it shortly
            rememberOrphanStream(track.id, stream, track.kind);
            rememberOrphanStream(stream.id, stream, track.kind);
        };

        // Perfect-negotiation: drive (re)negotiation through a single guarded path.
        pc.onnegotiationneeded = async () => {
            console.log('[VOICE] onnegotiationneeded fired');
            try {
                makingOfferRef.current = true;
                // Force all media types to be negotiated even if no tracks yet.
                // This ensures the SDP has m= lines ready for when tracks activate.
                const _offer = await pc.createOffer({ offerToReceiveAudio: true, offerToReceiveVideo: true });
                await pc.setLocalDescription(_offer);
                if (_offer.sdp) {
                    console.log('[VOICE] sending offer (sdp size=' + _offer.sdp.length + ')');
                    sendSignal({ type: 'offer', sdp: _offer.sdp } as any);
                } else {
                    console.warn('[VOICE] createOffer returned empty sdp!');
                }
            } catch (err) {
                console.error('[VOICE] RTC negotiation error:', err);
            } finally {
                makingOfferRef.current = false;
            }
        };
    }, [sendSignal, localStreamRef, setRemoteStreams, setRemoteVideoStreams]);

    /** Dispatches every incoming server signal to the appropriate state updater. */
    const handleMessage = useCallback(async (data: string) => {
        try {
            const msg = JSON.parse(data) as ServerSignal;
            console.log('[VOICE] handleMessage type=' + msg.type, msg);
            switch (msg.type) {
                case 'joined':
                    console.log('[VOICE] JOINED channel=' + msg.channelId + ' peers=' + (msg.peers?.length ?? 0));
                    if (msg.channelId !== 'global') {
                        const peers = msg.peers.map((p: any) => ({
                            ...p, isMuted: !!p.isMuted, isDeafened: !!p.isDeafened,
                        }));
                        setParticipants([
                            { userId: userIdRef.current, username: usernameRef.current, isMuted: false, isDeafened: false },
                            ...peers,
                        ]);
                        setChannelStartedAt(msg.startedAt);
                        connectSFU();
                    }
                    break;
                case 'peer-joined': {
                    // Suppress global presence toasts — they leak presence of any
                    // user (incl. non-friends) and create noise outside vocal rooms.
                    if (msg.channelId === 'global') break;
                    const peer = { ...msg.peer, isMuted: !!msg.peer.isMuted, isDeafened: !!msg.peer.isDeafened };
                    setParticipants(p => p.some(part => part.userId === peer.userId) ? p : [...p, peer]);
                    addToast(`${msg.peer.username} a rejoint le salon`, 'join');
                    break;
                }
                case 'peer-left':
                    setParticipants(p => {
                        const _leaving = p.find(part => part.userId === msg.userId);
                        if (_leaving && msg.channelId !== 'global') {
                            addToast(`${_leaving.username} a quitté le salon`, 'leave');
                        }
                        return p.filter(part => part.userId !== msg.userId);
                    });
                    break;
                case 'peer-state':
                    setParticipants(p => p.map(part =>
                        part.userId === msg.userId ? { ...part, isMuted: msg.isMuted, isDeafened: msg.isDeafened } : part));
                    break;
                case 'track-map': {
                    console.log('[VOICE] track-map:', { trackId: msg.trackId, streamId: msg.streamId, userId: msg.userId, kind: msg.kind });
                    rememberTrackMap(msg.trackId, msg.userId);
                    rememberTrackMap(msg.streamId, msg.userId);

                    // Resolve any orphan stream that arrived before this mapping
                    const _orphan = orphanStreamsRef.current.get(msg.trackId)
                        ?? orphanStreamsRef.current.get(msg.streamId)
                        ?? findUniqueOrphanByKind(msg.kind);
                    if (_orphan) {
                        console.log('[VOICE] track-map resolving orphan:', { userId: msg.userId, kind: _orphan.kind });
                        if (_orphan.kind === 'audio') {
                            setRemoteStreams(r => new Map(r).set(msg.userId, _orphan.stream));
                        } else if (_orphan.kind === 'video') {
                            setRemoteVideoStreams(r => new Map(r).set(msg.userId, _orphan.stream));
                        }
                        deleteOrphanStream(msg.trackId);
                        deleteOrphanStream(msg.streamId);
                    } else {
                        // Buffer the mapping for an `ontrack` that hasn't fired yet
                        console.log('[VOICE] track-map buffering for future ontrack');
                        rememberPendingTrackMap(msg.trackId, msg.userId, msg.kind);
                        rememberPendingTrackMap(msg.streamId, msg.userId, msg.kind);
                    }

                    // Migrate any state entry keyed by streamId / trackId to userId
                    setRemoteStreams(prev => {
                        const _src = prev.get(msg.streamId) ?? prev.get(msg.trackId);
                        if (!_src) return prev;
                        console.log('[VOICE] track-map migrating audio stream from', msg.streamId || msg.trackId, 'to', msg.userId);
                        const _next = new Map(prev);
                        _next.set(msg.userId, _src);
                        _next.delete(msg.streamId);
                        _next.delete(msg.trackId);
                        return _next;
                    });
                    setRemoteVideoStreams(prev => {
                        const _src = prev.get(msg.streamId) ?? prev.get(msg.trackId);
                        if (!_src) return prev;
                        console.log('[VOICE] track-map migrating video stream from', msg.streamId || msg.trackId, 'to', msg.userId);
                        const _next = new Map(prev);
                        _next.set(msg.userId, _src);
                        _next.delete(msg.streamId);
                        _next.delete(msg.trackId);
                        return _next;
                    });
                    break;
                }
                case 'offer': {
                    if (!sfuConnectionRef.current) await connectSFU();
                    const _pc = sfuConnectionRef.current!;

                    // The SFU sends `sdp` as a raw SDP string (not a full
                    // RTCSessionDescriptionInit). Wrap it client-side. Using
                    // a plain init object also avoids the deprecated
                    // RTCSessionDescription constructor, which silently
                    // produced a broken description in modern browsers and
                    // prevented DTLS from completing — the actual reason
                    // remote audio/video was never delivered.

                    const _offerInit: RTCSessionDescriptionInit = { type: 'offer', sdp: msg.sdp };

                    // Polite-peer collision detection: a glare happens when a
                    // remote offer arrives while a local offer is in-flight or
                    // the PC is not in a clean `stable` state. The SFU is
                    // impolite, so we always accept its offer, performing a
                    // rollback when needed (browsers ≥ M80 / Firefox ≥ 75).

                    const _readyForOffer = !makingOfferRef.current
                        && (_pc.signalingState === 'stable' || isSettingRemoteAnswerPendingRef.current);
                    const _offerCollision = !_readyForOffer;
                    ignoreOfferRef.current = false; // polite — never ignore
                    if (_offerCollision) {
                        await Promise.all([
                            _pc.setLocalDescription({ type: 'rollback' }).catch(() => {}),
                            _pc.setRemoteDescription(_offerInit),
                        ]);
                    } else {
                        await _pc.setRemoteDescription(_offerInit);
                    }
                    const _answer = await _pc.createAnswer();
                    await _pc.setLocalDescription(_answer);
                    // Send raw SDP string (see comment in onnegotiationneeded).
                    if (_answer.sdp) {
                        sendSignal({ type: 'answer', sdp: _answer.sdp } as any);
                    }
                    break;
                }
                case 'answer': {
                    if (!sfuConnectionRef.current) break;
                    const _pc = sfuConnectionRef.current;
                    if (_pc.signalingState !== 'have-local-offer') {
                        // Stale answer (already rolled-back or superseded). Drop it
                        // rather than crash setRemoteDescription.
                        break;
                    }
                    // Same wire-format note as for 'offer': `msg.sdp` is a string.
                    const _answerInit: RTCSessionDescriptionInit = { type: 'answer', sdp: msg.sdp };
                    isSettingRemoteAnswerPendingRef.current = true;
                    try {
                        await _pc.setRemoteDescription(_answerInit);
                    } finally {
                        isSettingRemoteAnswerPendingRef.current = false;
                    }
                    break;
                }
                case 'ice':
                    if (sfuConnectionRef.current && msg.candidate) {
                        try {
                            await sfuConnectionRef.current.addIceCandidate(new RTCIceCandidate(msg.candidate));
                        } catch (err) {
                            // Tolerate ICE errors during rollback — they are expected
                            // and would otherwise spam the console.
                            if (!ignoreOfferRef.current) console.warn('ICE add failed:', err);
                        }
                    }
                    break;
                case 'chat':
                    setChatMessages(prev => {
                        const id = `${msg.from}-${msg.timestamp}`;
                        if (prev.some(m => m.id === id)) return prev;
                        return [...prev, { id, from: msg.from, username: msg.username, message: msg.message, timestamp: Number(msg.timestamp), channelId: msg.channelId }];
                    });
                    // Fan out to bus subscribers (ChatContext + future consumers).
                    emitSignalingEvent('chat', {
                        id: `${msg.from}-${msg.timestamp}`,
                        from: msg.from,
                        username: msg.username,
                        message: msg.message,
                        timestamp: Number(msg.timestamp),
                        channelId: msg.channelId,
                    });
                    break;
                case 'stats':
                    setBandwidthStats(prev => new Map(prev).set(msg.userId, msg.bandwidthBps));
                    break;
                case 'error':
                    console.error('[VOICE] SERVER ERROR →', msg.message);
                    setError(msg.message);
                    break;
                case 'friend-request-received':
                case 'friend-request-accepted':
                case 'friend-request-declined':
                case 'friend-request-cancelled':
                case 'friend-removed':
                    emitSignalingEvent(msg.type, msg as never);
                    break;
                case 'ice-servers':
                    // Received once right after 'authenticated'. Store for the
                    // *next* connectSFU() — if a PC is already open (rare race:
                    // reconnect mid-call) it keeps its current ICE config until
                    // the following renegotiation/reconnect picks up the new one.
                    console.log('[VOICE] ice-servers received:', msg.servers.map(s => s.urls));
                    dynamicIceServersRef.current = msg.servers;
                    break;
                case 'authenticated':
                case 'server-member-presence':
                case 'server-member-added':
                case 'server-member-removed':
                case 'rpc-result':
                case 'dm-message':
                case 'dm-ack':
                    // Phase 3 push events — forwarded verbatim to the bus.
                    emitSignalingEvent(msg.type, msg as never);
                    break;
            }
        } catch (e) { console.error("Signal parsing error:", e); }
    }, [connectSFU, sendSignal, addToast, userIdRef, usernameRef, setParticipants, setChannelStartedAt, setRemoteStreams, setRemoteVideoStreams, setChatMessages, setBandwidthStats, setError]);

    return {
        sfuConnectionRef, screenStreamRef,
        connectSFU, handleMessage,
        addScreenTrack, removeScreenTrack,
    };
}