import { Injectable, Injector } from '@angular/core';
import {
    HttpInterceptor, HttpRequest, HttpHandler, HttpErrorResponse,
    HttpEvent
} from '@angular/common/http';
import { getJwtToken, getRefreshToken } from '../utils';
import { catchError, filter, finalize, switchMap, take } from 'rxjs/operators';
import { MatSnackBar } from '@angular/material/snack-bar';
import { BehaviorSubject, Observable, throwError } from 'rxjs';
import { ProgressiveLoaderService } from '../services/progressive-loader.service';
import { ApiService } from '../services/api.service';
import { Router } from '@angular/router';

@Injectable()
export class AuthInterceptor implements HttpInterceptor {
    private isRefreshing = false;
    private refreshTokenSubject = new BehaviorSubject<string | null>(null);
    private apiService!: ApiService;

    constructor(
        private snackBar: MatSnackBar,
        private loader: ProgressiveLoaderService,
        private injector: Injector,
        private router: Router
    ) {}

    private getApiService(): ApiService {
        if (!this.apiService) {
            this.apiService = this.injector.get(ApiService);
        }
        return this.apiService;
    }

    intercept(req: HttpRequest<any>, next: HttpHandler): Observable<HttpEvent<any>> {
        this.loader.updateLoader(true);

        // Don't attach auth header to refresh endpoint (avoid circular)
        const isRefreshRequest = req.url.includes('/oauth/refresh');
        let authReq = req;
        if (!isRefreshRequest) {
            const token = getJwtToken();
            if (token) {
                authReq = req.clone({
                    headers: req.headers.set('Authorization', `Bearer ${token}`)
                });
            }
        }

        return next.handle(authReq).pipe(
            catchError((error: HttpErrorResponse) => {
                const isAuthEndpoint = req.url.includes('/oauth');
                if (error.status === 401 && !isAuthEndpoint) {
                    return this.handle401Error(req, next);
                }

                if (error.error?.error) {
                    this.snackBar.open(error.error.error, 'Dismiss', { duration: 5000 });
                }

                return throwError(() => error);
            }),
            finalize(() => {
                this.loader.updateLoader(false);
            })
        );
    }

    private handle401Error(req: HttpRequest<any>, next: HttpHandler): Observable<HttpEvent<any>> {
        if (!this.isRefreshing) {
            this.isRefreshing = true;
            this.refreshTokenSubject.next(null);

            const refreshToken = getRefreshToken();
            if (!refreshToken) {
                this.forceLogout();
                return throwError(() => new Error('No refresh token'));
            }

            return this.getApiService().refreshToken(refreshToken).pipe(
                switchMap((response) => {
                    this.isRefreshing = false;
                    localStorage.setItem('vm_jwt', response.jwt);
                    localStorage.setItem('vm_refresh_token', response.refresh_token);
                    this.refreshTokenSubject.next(response.jwt);

                    // Retry original request with new token
                    return next.handle(req.clone({
                        headers: req.headers.set('Authorization', `Bearer ${response.jwt}`)
                    }));
                }),
                catchError((err) => {
                    this.isRefreshing = false;
                    this.forceLogout();
                    return throwError(() => err);
                })
            );
        } else {
            // Wait for in-progress refresh to complete, then retry
            return this.refreshTokenSubject.pipe(
                filter(token => token !== null),
                take(1),
                switchMap((token) => {
                    return next.handle(req.clone({
                        headers: req.headers.set('Authorization', `Bearer ${token}`)
                    }));
                })
            );
        }
    }

    private forceLogout(): void {
        localStorage.removeItem('vm_pubkey');
        localStorage.removeItem('vm_jwt');
        localStorage.removeItem('vm_refresh_token');
        this.router.navigate(['/login']);
    }
}
