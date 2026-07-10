// Copyright (c) 2025 Raphael Taibi. All rights reserved.
// Licensed under the Business Source License 1.1 (BUSL-1.1).
// See LICENSE file at repository root. Change Date: 2031-04-07.
// SPDX-License-Identifier: BUSL-1.1

import { useState, useCallback } from 'react';

const STORAGE_KEY = 'void_experimental_enabled';

/**
 * Manages the experimental features toggle.
 * State is persisted to localStorage so it survives reloads.
 *
 * @returns The current toggle state and a setter.
 */
export const useExperimentalSettings = () => {
    const [experimentalEnabled, setExperimentalEnabledState] = useState<boolean>(
        () => localStorage.getItem(STORAGE_KEY) === 'true',
    );

    const setExperimentalEnabled = useCallback((value: boolean) => {
        localStorage.setItem(STORAGE_KEY, String(value));
        setExperimentalEnabledState(value);
    }, []);

    return { experimentalEnabled, setExperimentalEnabled };
};

