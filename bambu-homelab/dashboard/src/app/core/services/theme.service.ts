import { Injectable, signal } from '@angular/core';

export type Theme = 'dark' | 'light';

const STORAGE_KEY = 'bambu-theme';

@Injectable({ providedIn: 'root' })
export class ThemeService {
  readonly theme = signal<Theme>(this.initial());

  toggle() {
    const next = this.theme() === 'dark' ? 'light' : 'dark';
    this.theme.set(next);
    document.documentElement.setAttribute('data-theme', next);
    localStorage.setItem(STORAGE_KEY, next);
  }

  /** Apply stored theme to DOM — called once at bootstrap. */
  init() {
    document.documentElement.setAttribute('data-theme', this.theme());
  }

  private initial(): Theme {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'light' || stored === 'dark') return stored;
    return 'dark';
  }
}
