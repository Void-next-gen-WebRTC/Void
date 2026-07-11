import { useState, useRef, useEffect, useCallback } from 'react';
import { searchUsers } from '../api/auth.api';
import { UserSummary } from '../models/auth/serverAuth.model';

const POPOVER_WIDTH = 300;
const POPOVER_HEIGHT = 320;
const MARGIN = 8;
const DEBOUNCE_MS = 350;
const MIN_QUERY_LENGTH = 2;

/** Matches a full tag `pseudo#XXXX` (suffix = 2-8 alphanumeric chars). */
const TAG_REGEX = /^.+#[A-Za-z0-9]{2,8}$/;
/** Matches a public-key prefix or full key (base64-ish, at least 8 chars). */
const PUBKEY_REGEX = /^[A-Za-z0-9+/=_-]{8,}$/;

/**
 * Encapsulates all state and side-effects for the AddFriendPopover.
 * @param onSend - Callback fired when the user sends a friend request.
 */
export function useAddFriendPopover(onSend: (userId: string) => void) {
    const [isOpen, setIsOpen] = useState(false);
    const [query, setQuery] = useState('');
    const [results, setResults] = useState<UserSummary[]>([]);
    const [loading, setLoading] = useState(false);
    const [sent, setSent] = useState<string | null>(null);
    const [pos, setPos] = useState({ top: 0, left: 0 });

    const btnRef = useRef<HTMLButtonElement>(null);
    const popoverRef = useRef<HTMLDivElement>(null);
    const inputRef = useRef<HTMLInputElement>(null);
    const _debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

    /** Computes the popover position relative to the trigger button. */
    const computePosition = useCallback(() => {
        if (!btnRef.current) return;
        const rect = btnRef.current.getBoundingClientRect();
        let top = rect.bottom + MARGIN;
        let left = rect.left + rect.width / 2 - POPOVER_WIDTH / 2;
        left = Math.max(MARGIN, Math.min(left, window.innerWidth - POPOVER_WIDTH - MARGIN));
        if (top + POPOVER_HEIGHT > window.innerHeight) top = rect.top - POPOVER_HEIGHT - MARGIN;
        setPos({ top, left });
    }, []);

    const handleToggle = useCallback(() => {
        if (!isOpen) computePosition();
        setIsOpen(prev => !prev);
        setQuery('');
        setResults([]);
        setSent(null);
    }, [isOpen, computePosition]);

    /* Auto-focus the input when the popover opens */
    useEffect(() => {
        if (isOpen) inputRef.current?.focus();
    }, [isOpen]);

    /* Close on outside click */
    useEffect(() => {
        if (!isOpen) return;
        const handler = (e: MouseEvent) => {
            const _target = e.target as Node;
            if (
                popoverRef.current && !popoverRef.current.contains(_target) &&
                btnRef.current && !btnRef.current.contains(_target)
            ) {
                setIsOpen(false);
            }
        };
        document.addEventListener('mousedown', handler);
        return () => document.removeEventListener('mousedown', handler);
    }, [isOpen]);

    const doSearch = useCallback(async (q: string) => {
        const _trimmed = q.trim();
        if (_trimmed.length < MIN_QUERY_LENGTH) { setResults([]); return; }
        // Only fire the API call for valid tag (Pseudo#XXXX) or public key inputs
        // to avoid noisy partial matches by displayName alone.
        const _isTag = TAG_REGEX.test(_trimmed);
        const _isPubkey = PUBKEY_REGEX.test(_trimmed) && !_trimmed.includes('#');
        if (!_isTag && !_isPubkey) { setResults([]); return; }
        setLoading(true);
        try {
            const _users = await searchUsers(_trimmed);
            setResults(_users);
        } catch (err) { console.warn('search failed:', err); setResults([]); }
        finally { setLoading(false); }
    }, []);

    const handleInputChange = useCallback((val: string) => {
        setQuery(val);
        setSent(null);
        if (_debounceRef.current) clearTimeout(_debounceRef.current);
        _debounceRef.current = setTimeout(() => doSearch(val), DEBOUNCE_MS);
    }, [doSearch]);

    const handleSend = useCallback((userId: string) => {
        onSend(userId);
        setSent(userId);
    }, [onSend]);

    /** True when the input is non-empty but doesn't match a tag/pubkey format. */
    const isInvalidFormat = (() => {
        const _trimmed = query.trim();
        if (_trimmed.length < MIN_QUERY_LENGTH) return false;
        return !TAG_REGEX.test(_trimmed)
            && !(PUBKEY_REGEX.test(_trimmed) && !_trimmed.includes('#'));
    })();

    return {
        isOpen,
        query,
        results,
        loading,
        sent,
        pos,
        btnRef,
        popoverRef,
        inputRef,
        isInvalidFormat,
        handleToggle,
        handleInputChange,
        handleSend,
    };
}


