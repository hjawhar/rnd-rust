import { Component } from '@angular/core';
import { Router } from '@angular/router';
import { takeUntil } from 'rxjs';
import { environment } from '../../../environments/environment';
import { BasePageComponent } from '../../core/base-page.component';
import { FormGroup, FormControl } from '@angular/forms';
import { isEmptyOrNull } from '../../shared/utils';
import { ApiService } from '../../shared/services/api.service';
import { HelperService } from '../../shared/services/helper.service';
import { RequestsService } from '../../shared/services/requests.service';
import { SharedModule } from '../../shared/shared.module';
import { CommonModule } from '@angular/common';
import { Buffer } from 'buffer';

declare let window: any;

@Component({
  selector: 'app-login',
  templateUrl: './login.component.html',
  styleUrl: './login.component.scss',
  imports: [CommonModule, SharedModule]
})
export class LoginComponent extends BasePageComponent {
  loginForm = new FormGroup({
    address: new FormControl(''),
    nonce: new FormControl(''),
    signed_nonce: new FormControl('')
  });

  isEntered = false;
  isMetaMaskConnect = false;

  msg?: string;
  nonce?: string;
  constructor(public helperService: HelperService, private router: Router, private req: RequestsService, private apiService: ApiService) {
    super();
  }

  isMetaMaskClicked = false;
  isLoginClicked = false;

  async onMetamaskLogin() {
    this.isMetaMaskClicked = true;
    await this.initWindowEthereumReq();
  }

  async onClickLogin() {
    this.isLoginClicked = true;
    await this.sendReq();
  }

  onEnter() {
    this.isEntered = true;
  }

  onMetaMaskConnect() {
    this.isMetaMaskConnect = true;
  }

  async initWindowEthereumReq() {
    try {
      const accounts = await window.ethereum.request({ method: 'eth_requestAccounts' });
      const walletAddress = accounts && Array.isArray(accounts) && accounts[0] ? accounts[0] : null;
      if (!walletAddress) {
        return;
      }
      localStorage.setItem('currentWallet', walletAddress);
      this.apiService.getNonce(walletAddress).pipe(takeUntil(this.componentDestroyed$)).subscribe(async response => {
        this.msg = `0x${Buffer.from(response.nonce, 'utf8').toString('hex')}`;
        this.nonce = response.nonce;
      });
    } catch (error) {
      return;
    }
  }

  async sendReq() {
    let currentWallet = localStorage.getItem('currentWallet');
    if (!isEmptyOrNull(currentWallet)) {
      // try {
      //   const signature: string = await window.ethereum.request({
      //     from: currentWallet,
      //     method: 'personal_sign',
      //     params: [this.msg, currentWallet],
      //   });
      // } catch (error) {
      //   console.log(error);
      // }

      const signature: string = await window.ethereum.request({
        from: currentWallet,
        method: 'personal_sign',
        params: [this.msg, currentWallet],
      });
      let data: {
        address?: string | null,
        nonce?: string | null,
        signed_nonce?: string | null
      } = {};
      data['address'] = currentWallet;
      data['nonce'] = this.nonce;
      data['signed_nonce'] = signature;
      const loginUrl = `${environment.baseUrl}/oauth`;
      this.req.post(loginUrl, data).subscribe(result => {
        localStorage.setItem('jwt', result.jwt);
        this.router.navigate(['/']);
      });
    }
  }
} 