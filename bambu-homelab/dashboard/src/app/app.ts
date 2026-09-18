import { Component, inject } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive } from '@angular/router';
import { WebSocketService } from './core/services/websocket.service';
import { AuthService } from './core/services/auth.service';
import { NotificationService } from './core/services/notification.service';
import { ThemeService } from './core/services/theme.service';

@Component({
  selector: 'app-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive],
  template: `
    <div class="min-h-screen bg-surface">
      <nav class="sticky top-0 z-40 bg-surface-light/80 backdrop-blur-xl border-b border-border">
        <div class="px-4 h-12 flex items-center justify-between">
          <!-- Left: Logo + Nav -->
          <div class="flex items-center gap-6">
            <a routerLink="/" class="flex items-center gap-2 group">
              <div class="w-7 h-7 rounded-lg bg-primary flex items-center justify-center">
                <svg class="w-4 h-4 text-white" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5"/>
                </svg>
              </div>
              <span class="text-sm font-semibold text-text group-hover:text-primary transition-colors">Bambu Homelab</span>
            </a>

            @if (auth.isAuthenticated()) {
              <div class="hidden sm:flex items-center gap-1">
                <a routerLink="/" routerLinkActive="bg-primary/10 text-primary" [routerLinkActiveOptions]="{exact: true}"
                   class="px-3 py-1.5 rounded-md text-xs font-medium text-text-muted hover:text-text hover:bg-surface-lighter/50 transition-colors">
                  Printers
                </a>
                @if (auth.isAdmin()) {
                <a routerLink="/fleet" routerLinkActive="bg-primary/10 text-primary"
                   class="px-3 py-1.5 rounded-md text-xs font-medium text-text-muted hover:text-text hover:bg-surface-lighter/50 transition-colors">
                  Fleet
                </a>
                }
                @if (auth.isAdmin()) {
                <a routerLink="/admin/users" routerLinkActive="bg-primary/10 text-primary"
                   class="px-3 py-1.5 rounded-md text-xs font-medium text-text-muted hover:text-text hover:bg-surface-lighter/50 transition-colors">
                  Users
                </a>
                }
              </div>
            }
          </div>

          <!-- Right: Status + Theme + User -->
          <div class="flex items-center gap-2">
            <!-- Connection status -->
            @if (ws.connected()) {
              <div class="flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] text-success" title="WebSocket connected">
                <span class="w-1.5 h-1.5 rounded-full bg-success"></span>
                <span class="hidden md:inline">Connected</span>
              </div>
            } @else {
              <div class="flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] text-warning" title="Reconnecting...">
                <span class="w-1.5 h-1.5 rounded-full bg-warning animate-pulse"></span>
                <span class="hidden md:inline">Reconnecting</span>
              </div>
            }

            <!-- Theme toggle -->
            <button (click)="themeService.toggle()"
                    class="p-1.5 rounded-md text-text-muted hover:text-text hover:bg-surface-lighter/50 transition-colors"
                    [title]="themeService.theme() === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'">
              @if (themeService.theme() === 'dark') {
                <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                  <circle cx="12" cy="12" r="5"/>
                  <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/>
                </svg>
              } @else {
                <svg class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                  <path d="M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z"/>
                </svg>
              }
            </button>

            @if (auth.isAuthenticated()) {
              <div class="w-px h-5 bg-border"></div>

              <!-- User -->
              <div class="flex items-center gap-2">
                <div class="w-6 h-6 rounded-full bg-primary/20 text-primary flex items-center justify-center text-[10px] font-bold uppercase">
                  {{ (auth.username() ?? '?').charAt(0) }}
                </div>
                <span class="text-xs text-text-muted hidden lg:inline">{{ auth.username() }}</span>
                <button (click)="auth.logout()"
                        class="p-1.5 rounded-md text-text-muted hover:text-error hover:bg-error/10 transition-colors" title="Logout">
                  <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"/>
                  </svg>
                </button>
              </div>
            }
          </div>
        </div>
      </nav>

      <main>
        <router-outlet />
      </main>
    </div>
  `,
})
export class AppComponent {
  ws = inject(WebSocketService);
  auth = inject(AuthService);
  themeService = inject(ThemeService);
  private notifications = inject(NotificationService);

  constructor() {
    this.notifications.init();
    this.themeService.init();
  }
}
