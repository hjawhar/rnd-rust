import { Injectable, inject, signal, computed } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { Router } from '@angular/router';
import { Observable, tap } from 'rxjs';
import { LoginRequest, LoginResponse, ChangePasswordRequest } from '../models/auth.model';

const TOKEN_KEY = 'bambu_token';
const USER_KEY = 'bambu_user';
const ROLE_KEY = 'bambu_role';

@Injectable({ providedIn: 'root' })
export class AuthService {
  private http = inject(HttpClient);
  private router = inject(Router);

  private _token = signal<string | null>(localStorage.getItem(TOKEN_KEY));
  private _username = signal<string | null>(localStorage.getItem(USER_KEY));
  private _role = signal<string | null>(localStorage.getItem(ROLE_KEY));

  readonly isAuthenticated = computed(() => !!this._token());
  readonly username = this._username.asReadonly();
  readonly token = this._token.asReadonly();
  readonly role = this._role.asReadonly();
  readonly isAdmin = computed(() => this._role() === 'admin');

  login(req: LoginRequest): Observable<LoginResponse> {
    return this.http.post<LoginResponse>('/api/auth/login', req).pipe(
      tap(res => {
        localStorage.setItem(TOKEN_KEY, res.token);
        localStorage.setItem(USER_KEY, res.username);
        localStorage.setItem(ROLE_KEY, res.role);
        this._token.set(res.token);
        this._username.set(res.username);
        this._role.set(res.role);
      }),
    );
  }

  logout(): void {
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
    localStorage.removeItem(ROLE_KEY);
    this._token.set(null);
    this._username.set(null);
    this._role.set(null);
    this.router.navigate(['/login']);
  }

  changePassword(req: ChangePasswordRequest): Observable<void> {
    return this.http.post<void>('/api/auth/password', req);
  }
}
