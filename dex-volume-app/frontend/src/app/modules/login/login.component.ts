import { AfterViewInit, Component } from '@angular/core';
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
import bs58 from 'bs58';

declare let window: any;

@Component({
  selector: 'app-login',
  templateUrl: './login.component.html',
  styleUrl: './login.component.scss',
  imports: [CommonModule, SharedModule]
})
export class LoginComponent extends BasePageComponent implements AfterViewInit {
  loginForm = new FormGroup({
    address: new FormControl(''),
    nonce: new FormControl(''),
    signed_nonce: new FormControl('')
  });

  msg?: string;
  nonce?: string;
  constructor(public helperService: HelperService, private router: Router, private apiService: ApiService) {
    super();
  }

  async ngAfterViewInit() {
    let provider = this.getProvider();
    provider.on("connect", async (publicKey: string) => {
      console.log(`Wallet ${publicKey} connected`);
      localStorage.setItem('vm_pubkey', publicKey.toString());
    });

    provider.on('accountChanged', (publicKey: any) => {
      if (publicKey) {
        // Set new public key and continue as usual
        console.log(`Switched to account ${publicKey.toBase58()}`);
      } else {
        // Attempt to reconnect to Phantom
        provider.connect().catch((error: any) => {
          // Handle connection failure
        });
      }
    });

    try {
      const resp = await provider.connect();
    } catch (err) {
      // { code: 4001, message: 'User rejected the request.' }
    }
  }

  isPhantomClicked = false;
  isLoginClicked = false;

  async onPhantomLogin() {
    this.isPhantomClicked = true;
    await this.initPhantom();
  }

  async onClickLogin() {
    this.isLoginClicked = true;
    await this.sendReq();
  }

  async initPhantom() {
    try {
      try {
        const resp = await this.getProvider().connect();
      } catch (err) {
      }
    } catch (error) {
      return;
    }
  }

  async sendReq() {
    let currentWallet = localStorage.getItem('vm_pubkey');
    if (!isEmptyOrNull(currentWallet)) {
      this.apiService.getNonce(currentWallet!).subscribe(async ({ nonce, address }) => {
        let signature = await this.signMessage(nonce!);

        this.apiService.authenticate(currentWallet!, nonce, signature).subscribe(({ jwt, refresh_token }) => {
          localStorage.setItem('vm_jwt', jwt);
          localStorage.setItem('vm_refresh_token', refresh_token);
          this.router.navigate(['/']);
        })
      });
    }
  }




  getProvider() {
    if ('phantom' in window) {
      const provider = (window.phantom! as any).solana;
      if (provider?.isPhantom) {
        return provider;
      }
    }

    window.open('https://phantom.app/', '_blank');
    return null;
  };

  async signMessage(message: string) {
    let provider = this.getProvider();
    const encodedMessage = new TextEncoder().encode(message);
    const signedMessage = await provider.signMessage(encodedMessage, "utf8");
    const signedMessageString = bs58.encode(signedMessage.signature);
    return signedMessageString;
  }
} 