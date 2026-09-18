import { Component, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { AuthService } from '../../core/services/auth.service';

@Component({
  selector: 'app-login',
  imports: [FormsModule],
  template: `
    <div class="min-h-screen bg-surface flex items-center justify-center">
      <div class="w-full max-w-sm">
        <h1 class="text-3xl font-bold text-primary text-center mb-8">Bambu Homelab</h1>

        <form (ngSubmit)="login()" class="bg-surface-light rounded-lg border border-border p-6">
          <h2 class="text-lg font-semibold text-text mb-4">Sign In</h2>

          <div class="mb-4">
            <label class="block text-sm text-text-muted mb-1">Username</label>
            <input type="text" [(ngModel)]="username" name="username" required autofocus
                   class="w-full px-3 py-2 bg-surface-lighter border border-border rounded-lg text-text focus:border-primary focus:outline-none" />
          </div>

          <div class="mb-4">
            <label class="block text-sm text-text-muted mb-1">Password</label>
            <input type="password" [(ngModel)]="password" name="password" required
                   class="w-full px-3 py-2 bg-surface-lighter border border-border rounded-lg text-text focus:border-primary focus:outline-none" />
          </div>

          @if (error()) {
            <div class="mb-4 p-3 bg-error/10 border border-error/30 rounded-lg text-sm text-error">
              {{ error() }}
            </div>
          }

          <button type="submit" [disabled]="loading()"
                  class="w-full px-4 py-2 bg-primary text-white rounded-lg hover:bg-primary-dark transition-colors font-medium disabled:opacity-50">
            {{ loading() ? 'Signing in...' : 'Sign In' }}
          </button>
        </form>
      </div>
    </div>
  `,
})
export class LoginPage {
  private auth = inject(AuthService);
  private router = inject(Router);

  username = '';
  password = '';
  loading = signal(false);
  error = signal<string | null>(null);

  login() {
    this.error.set(null);
    this.loading.set(true);

    this.auth.login({ username: this.username, password: this.password }).subscribe({
      next: () => {
        this.router.navigate(['/']);
      },
      error: (err) => {
        this.loading.set(false);
        this.error.set(err.status === 401 ? 'Invalid username or password' : 'Login failed');
      },
    });
  }
}
