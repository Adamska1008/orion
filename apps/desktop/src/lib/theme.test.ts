import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyTheme, readThemePreference, saveThemePreference } from './theme';

afterEach(() => vi.unstubAllGlobals());

describe('theme preferences', () => {
  it('restores saved choices and falls back to system for missing or invalid values', () => {
    for (const [saved, expected] of [[null, 'system'], ['invalid', 'system'], ['system', 'system'], ['dark', 'dark'], ['light', 'light']]) {
      vi.stubGlobal('localStorage', { getItem: () => saved });
      expect(readThemePreference()).toBe(expected);
    }
  });

  it('persists the preference rather than the current system appearance', () => {
    const setItem = vi.fn();
    vi.stubGlobal('localStorage', { setItem });
    saveThemePreference('system');
    expect(setItem).toHaveBeenCalledWith('orion.theme', 'system');
  });

  it('still works when storage is blocked or full', () => {
    vi.stubGlobal('localStorage', {
      getItem: () => { throw new Error('blocked'); },
      setItem: () => { throw new Error('full'); },
    });
    expect(readThemePreference()).toBe('system');
    expect(() => saveThemePreference('dark')).not.toThrow();
  });

  it('tracks system appearance while preserving explicit overrides', () => {
    const dataset: Record<string, string> = {};
    const setAttribute = vi.fn();
    const media = { matches: true };
    vi.stubGlobal('window', { matchMedia: () => media });
    vi.stubGlobal('document', { documentElement: { dataset }, querySelector: () => ({ setAttribute }) });
    applyTheme('system');
    expect(dataset.theme).toBe('dark');
    expect(setAttribute).toHaveBeenLastCalledWith('content', '#14171d');
    media.matches = false;
    applyTheme('system');
    expect(dataset.theme).toBe('light');
    expect(setAttribute).toHaveBeenLastCalledWith('content', '#f6f7f9');
    applyTheme('dark');
    expect(dataset.theme).toBe('dark');
    media.matches = true;
    applyTheme('light');
    expect(dataset.theme).toBe('light');
  });
});
