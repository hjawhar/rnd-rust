import { Injectable } from '@angular/core';
import { isLoggedIn } from '../utils';
import { ApiService } from './api.service';
import { RequestsService } from './requests.service';
import { ActivatedRoute, Router } from '@angular/router';
declare let window: any;

@Injectable({
    providedIn: 'root'
})
export class HelperService {
    isLoggedIn = isLoggedIn;
    constructor(private route: ActivatedRoute, private req: RequestsService, private apiService: ApiService, private router: Router) {
    }

    public getWalletAddress() {
        return localStorage.getItem('vm_pubkey');
    }

    public logout() {
        this.apiService.logout().subscribe({ error: () => {} });
        localStorage.removeItem('vm_pubkey');
        localStorage.removeItem('vm_jwt');
        localStorage.removeItem('vm_refresh_token');
        this.router.navigate(['/login']);
    }

    
}
