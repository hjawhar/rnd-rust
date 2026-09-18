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

    async initMetamask() {
        try {
            const accounts = await window.ethereum.request({ method: 'eth_requestAccounts' });
            const walletAddress = accounts && Array.isArray(accounts) && accounts[0] ? accounts[0] : null;
            if (!walletAddress) {
                return;
            }
            localStorage.setItem('currentWallet', walletAddress);
        } catch (error) {
            return;
        }
        window.ethereum.on('accountsChanged', (accounts: Array<string>) => {
            if (accounts.length > 0) {
                localStorage.setItem('currentWallet', accounts[0]);
            }
        });
    }

    public getWalletAddress() {
        return localStorage.getItem('currentWallet');
    }

    public logout() {
        localStorage.removeItem('currentWallet');
        localStorage.removeItem('jwt');
        this.router.navigate(['/login']);
    }
}
