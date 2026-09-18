import { Injectable } from '@angular/core';
import { BehaviorSubject } from 'rxjs';

export type Theme = 'dark' | 'light';

@Injectable({
    providedIn: 'root'
})
export class ThemeService {
    private readonly THEME_KEY = 'vm_theme';
    private themeSubject = new BehaviorSubject<Theme>(this.getStoredTheme());

    theme$ = this.themeSubject.asObservable();

    constructor() {
        this.applyTheme(this.getStoredTheme());
    }

    private getStoredTheme(): Theme {
        const stored = localStorage.getItem(this.THEME_KEY) as Theme;
        return stored || 'dark';
    }

    get currentTheme(): Theme {
        return this.themeSubject.value;
    }

    toggleTheme(): void {
        const newTheme = this.currentTheme === 'dark' ? 'light' : 'dark';
        this.setTheme(newTheme);
    }

    setTheme(theme: Theme): void {
        localStorage.setItem(this.THEME_KEY, theme);
        this.themeSubject.next(theme);
        this.applyTheme(theme);
    }

    private applyTheme(theme: Theme): void {
        document.documentElement.setAttribute('data-theme', theme);
    }
}
