// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// See LICENSE file at repository root. Change Date: 2031-04-07.
// SPDX-License-Identifier: BUSL-1.1

import { AlertTriangle, FlaskConical } from 'lucide-react';
import { useExperimentalSettings } from '../../hooks/useExperimentalSettings';

/**
 * Experimental features settings panel.
 * Exposes a toggle to opt into QUIC/QMux prototype transport.
 * Displays a stability warning when the feature is enabled.
 */
export const ExperimentalSettings = () => {
    const { experimentalEnabled, setExperimentalEnabled } = useExperimentalSettings();

    return (
        <div className="flex flex-col gap-8 animate-in fade-in duration-500">
            <h2 className="text-cyan-50 text-[24px] font-black uppercase tracking-wider drop-shadow-[0_0_10px_rgba(34,211,238,0.5)]">
                Fonctionnalités Expérimentales
            </h2>

            <div className="glass border border-amber-500/20 p-6 rounded-xl shadow-[0_0_20px_rgba(245,158,11,0.05)] relative group">
                <div className="absolute inset-0 bg-gradient-to-br from-amber-900/10 to-transparent pointer-events-none rounded-xl" />

                <div className="flex items-center gap-3 mb-5">
                    <h3 className="text-amber-400/70 text-[11px] font-black uppercase tracking-widest">
                        Expérimental
                    </h3>
                    <span className="bg-amber-500/20 border border-amber-500/50 text-amber-400 text-[9px] font-black uppercase px-2 py-0.5 rounded shadow-[0_0_10px_rgba(245,158,11,0.3)]">
                        BETA
                    </span>
                </div>

                {/* QUIC / QMux toggle */}
                <div className="glass-light rounded-lg p-4 flex items-center justify-between hover:border-amber-500/30 transition-all group/card relative">
                    <div className="absolute left-0 top-0 bottom-0 w-1 bg-amber-500/30 group-hover/card:bg-amber-400 transition-colors rounded-l-lg" />
                    <div className="flex flex-col gap-1 pl-2">
                        <div className="flex items-center gap-2">
                            <FlaskConical size={15} className="text-amber-400" />
                            <span className="text-cyan-50 font-bold text-[14px]">Mode Expérimental</span>
                        </div>
                        <span className="text-cyan-500/50 text-[12px] font-medium">
                            Active les fonctionnalités QUIC et QMux en cours de développement.
                        </span>
                    </div>
                    <label className="relative inline-flex items-center cursor-pointer shrink-0 ml-4">
                        <input
                            type="checkbox"
                            className="sr-only peer"
                            checked={experimentalEnabled}
                            onChange={(e) => setExperimentalEnabled(e.target.checked)}
                        />
                        <div className="w-11 h-6 bg-[#1a1c24] border border-amber-500/20 rounded-full peer peer-checked:bg-amber-500 peer-checked:border-amber-400 after:content-[''] after:absolute after:top-0.5 after:left-0.5 after:bg-white after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:after:translate-x-5 shadow-[inset_0_0_10px_rgba(0,0,0,0.5)]" />
                    </label>
                </div>

                {/* Stability warning — shown only when enabled */}
                {experimentalEnabled && (
                    <div className="mt-4 flex items-start gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 px-4 py-3 animate-in fade-in slide-in-from-top-1 duration-300">
                        <AlertTriangle size={16} className="text-amber-400 shrink-0 mt-0.5" />
                        <p className="text-amber-200/80 text-[12px] font-medium leading-relaxed">
                            <span className="font-black text-amber-300">Test QUIC &amp; QMux en développement — non stable.</span>
                            {' '}Des instabilités réseau peuvent avoir lieu. Désactivez cette option si vous rencontrez des déconnexions ou des problèmes audio.
                        </p>
                    </div>
                )}
            </div>
        </div>
    );
};

