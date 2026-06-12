import { useEffect, useRef } from 'react';
import VoiceAudioRendererProps from '../../models/voice/voiceAudioRendererProps.model';
import { useVoiceStore } from '../../context/VoiceContext';

const PLAY_RETRY_MS = 250;
const PLAY_RETRY_COUNT = 3;

/**
 * Retries `audio.play()` a few times because some WebView engines briefly
 * reject autoplay right after track attachment.
 */
const safePlay = async (audio: HTMLAudioElement): Promise<void> => {
    for (let _i = 0; _i < PLAY_RETRY_COUNT; _i += 1) {
        try {
            if (audio.paused) await audio.play();
            return;
        } catch {
            await new Promise((resolve) => setTimeout(resolve, PLAY_RETRY_MS));
        }
    }
};

export const VoiceAudioRenderer = ({ stream, muted, peerId }: VoiceAudioRendererProps) => {
    const audioRef = useRef<HTMLAudioElement | null>(null);
    const { userVolumes } = useVoiceStore();
    const volume = userVolumes.get(peerId) ?? 1;

    useEffect(() => {
        const audio = audioRef.current;
        if (!audio || !stream) return;

        if (audio.srcObject !== stream) {
            audio.srcObject = stream;
        }
        
        const safeVolume = Math.max(0, Math.min(1, volume));

        audio.muted = muted || safeVolume === 0;
        audio.volume = safeVolume;

        const _attemptPlay = () => { void safePlay(audio); };
        _attemptPlay();

        const selectedSpeaker = localStorage.getItem('selectedSpeaker');
        if (selectedSpeaker && 'setSinkId' in audio) {
            (audio as any).setSinkId(selectedSpeaker).catch((err: any) => {
                console.warn("Impossible de changer le périphérique de sortie audio:", err);
                // Stale sink ids silently break playback on some systems.
                localStorage.removeItem('selectedSpeaker');
                (audio as any).setSinkId?.('default').catch(() => {});
            });
        }

        const _tracks = stream.getAudioTracks();
        const _onTrackState = () => _attemptPlay();
        _tracks.forEach((t) => {
            t.addEventListener('unmute', _onTrackState);
            t.addEventListener('ended', _onTrackState);
        });

        audio.addEventListener('canplay', _onTrackState);

        return () => {
            audio.removeEventListener('canplay', _onTrackState);
            _tracks.forEach((t) => {
                t.removeEventListener('unmute', _onTrackState);
                t.removeEventListener('ended', _onTrackState);
            });
        };
    }, [stream, muted, volume]);

    return (
        <audio 
            ref={audioRef} 
            autoPlay 
            playsInline 
            style={{ display: 'none' }} 
        />
    );
};
