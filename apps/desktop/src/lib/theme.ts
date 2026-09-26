import { useEffect, useLayoutEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

export type ThemePreference = 'system' | 'light' | 'dark';
const storageKey = 'orion.theme';
const systemQuery = '(prefers-color-scheme: dark)';

export function readThemePreference(): ThemePreference {
  try {
    const saved = localStorage.getItem(storageKey);
    if (saved === 'system' || saved === 'light' || saved === 'dark') return saved;
  } catch { /* Storage can be unavailable; use the default dark theme. */ }
  return 'dark';
}

export function saveThemePreference(preference: ThemePreference) {
  try { localStorage.setItem(storageKey, preference); }
  catch { /* Keep the in-memory preference when storage is unavailable. */ }
}

export function applyTheme(preference: ThemePreference) {
  const theme = preference === 'system' ? (window.matchMedia(systemQuery).matches ? 'dark' : 'light') : preference;
  document.documentElement.dataset.theme = theme;
  document.querySelector('meta[name="theme-color"]')?.setAttribute('content', theme === 'dark' ? '#14171d' : '#f6f7f9');
}

export function useTheme() {
  const [preference, setPreference] = useState<ThemePreference>(readThemePreference);
  useLayoutEffect(() => {
    applyTheme(preference);
    if (preference !== 'system') return;
    const media = window.matchMedia(systemQuery);
    const update = () => applyTheme('system');
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, [preference]);

  useEffect(() => {
    if (isTauri()) {
      // null restores OS tracking for the native window and title bar.
      void getCurrentWindow().setTheme(preference === 'system' ? null : preference)
        .catch(error => console.warn('Unable to update native window theme', error));
    }
  }, [preference]);

  function selectTheme(next: ThemePreference) {
    saveThemePreference(next);
    setPreference(next);
  }
  return [preference, selectTheme] as const;
}
