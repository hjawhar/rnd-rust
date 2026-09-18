import { Injectable } from '@angular/core';
import { CanActivate, ActivatedRouteSnapshot, RouterStateSnapshot, Router, CanLoad, UrlSegment, Route, CanMatch, UrlTree } from '@angular/router';
import { isLoggedIn } from '../utils';
import { Observable } from 'rxjs';
import { MatSnackBar } from '@angular/material/snack-bar';

@Injectable({
    providedIn: 'root',
})
export class AuthGuard implements CanMatch {
    constructor(private router: Router, private snackBar: MatSnackBar) { }
    canMatch(route: Route, segments: UrlSegment[]): boolean | UrlTree | Observable<boolean | UrlTree> | Promise<boolean | UrlTree> {
        if (isLoggedIn()) {
            return true;
        }
        this.snackBar.open('You do not have access, please log in', 'Dismiss', {
            duration: 5000,
        });
        this.router.navigate(['/login']);
        return false;
    }
}
