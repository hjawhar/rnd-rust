import { Injectable } from '@angular/core';
import { HttpInterceptor, HttpRequest, HttpHandler, HttpErrorResponse, HttpResponse, HttpEvent } from '@angular/common/http';
import { getJwtToken } from '../utils';
import { tap, catchError, finalize } from 'rxjs/operators';
import { MatSnackBar } from '@angular/material/snack-bar';
import { MatDialog } from '@angular/material/dialog';
import { of } from 'rxjs';
import { ProgressiveLoaderService } from '../services/progressive-loader.service';

@Injectable()
export class AuthInterceptor implements HttpInterceptor {

    constructor(private snackBar: MatSnackBar, private loader: ProgressiveLoaderService, private dialog: MatDialog) {
    }

    intercept(req: HttpRequest<any>, next: HttpHandler) {
        this.loader.updateLoader(true);
        const authToken = `Bearer ${getJwtToken()}`;
        const authReq = req.clone({
            headers: req.headers.set('Authorization', authToken)
        });

        return next.handle(authReq).pipe(
            tap((event: HttpEvent<any>) => {
                if (event instanceof HttpResponse) {
                    switch (event.status) {
                        case 200: // Ok
                            break;
                        case 201: // Created
                            break;
                        case 202: // Accepted
                            break;
                        default:
                            break;
                    }
                }
            }), catchError(response => {
                // if (response.status !== 403) {
                //     let displayed_error = response && response.error && response.error.error ? response.error.error : '';
                //     this.snackBar.open(displayed_error, 'Dismiss', {
                //         duration: 5000,
                //     });
                // }
                if (response instanceof HttpErrorResponse) {
                    let { error } = response.error;
                    switch (response.status) {
                        case 400:
                            this.snackBar.open(error, 'Dismiss', {
                                duration: 5000,
                            });
                            /**
                             * Bad Request
                             */
                            break;
                        case 401:
                            /**
                             * Unauthorized
                             */
                            // this.watcher.logout();
                            this.snackBar.open(error, 'Dismiss', {
                                duration: 5000,
                            });
                            break;
                        case 403:
                            // this.notification.showError(errorMessage, false, 5);
                            // this.dialog.open(BannedComponent, {
                            //     data: {
                            //         message: response.error
                            //     },
                            //     width: '500px'
                            // });
                            this.snackBar.open(error, 'Dismiss', {
                                duration: 5000,
                            });
                            break;
                        case 500:
                            /**
                             * Internal Server Error
                             */
                            break;
                        default:
                            if (response.error && response.error.validation && response.error.validation[0]) {
                                // this.notification.onError('Error', response.error.validation[0].errorMessage);
                            } else {
                                // this.notification.onError('Error', response.error.description);
                            }
                            break;
                    }
                }
                return of(response.message);
            }),
            finalize(() => {
                this.loader.updateLoader(false);
            }));
    }
}
