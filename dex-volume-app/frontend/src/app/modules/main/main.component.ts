import { CommonModule } from '@angular/common';
import { AfterViewInit, Component } from '@angular/core';
import { SharedModule } from '../../shared/shared.module';
import { RouterModule, RouterOutlet } from '@angular/router';
import { WatcherService } from '../../shared/services/watcher.service';
import { HelperService } from '../../shared/services/helper.service';
import { ThemeService } from '../../shared/services/theme.service';
import { addressAbrev, getJwtToken, getRefreshToken, isAdmin, isLoggedIn } from '../../shared/utils';
import { MatSnackBar } from '@angular/material/snack-bar';
import { environment } from '../../../environments/environment';
import { BasePageComponent } from '../../core/base-page.component';
import { ApiService } from '../../shared/services/api.service';
import { SystemStatus } from '../../shared/models/system-status.model';
import { catchError } from 'rxjs/operators';
import { of } from 'rxjs';

@Component({
  selector: 'app-main',
  templateUrl: './main.component.html',
  styleUrl: './main.component.scss',
  imports: [CommonModule, RouterOutlet, RouterModule, SharedModule],
})
export class MainComponent extends BasePageComponent implements AfterViewInit {
  addressAbrev = addressAbrev;
  isAdmin = isAdmin;
  w?: WebSocket;
  systemStatus: SystemStatus | null = null;
  statusError = false;
  pingMs: number | null = null;
  backendVersion: string | null = null;
  frontendVersion: string = environment.version;
  private pingInterval: any;
  private versionCheckInterval: any;

  constructor(private apiService: ApiService, private snackbar: MatSnackBar, public watcherService: WatcherService, public helperService: HelperService, public themeService: ThemeService) {
    super();
  }

  ngAfterViewInit(): void {
    this.initSocket();
    this.fetchInitialStatus();
    this.measurePing();
    this.pingInterval = setInterval(() => this.measurePing(), 15000);
    this.versionCheckInterval = setInterval(() => this.checkFrontendVersion(), 60000);
  }

  override ngOnDestroy(): void {
    super.ngOnDestroy();
    if (this.pingInterval) clearInterval(this.pingInterval);
    if (this.versionCheckInterval) clearInterval(this.versionCheckInterval);
  }

  private fetchInitialStatus(): void {
    this.apiService.getSystemStatus().pipe(
      catchError(() => { this.statusError = true; return of(null); })
    ).subscribe(status => {
      if (status) { this.systemStatus = status; this.statusError = false; }
    });
  }

  private measurePing(): void {
    const start = performance.now();
    this.apiService.ping().pipe(
      catchError(() => of(null))
    ).subscribe((res) => {
      this.pingMs = Math.round(performance.now() - start);
      if (res?.version) {
        if (this.backendVersion && this.backendVersion !== res.version) {
          location.reload();
          return;
        }
        this.backendVersion = res.version;
      }
    });
  }

  private checkFrontendVersion(): void {
    this.apiService.checkFrontendVersion().pipe(
      catchError(() => of(null))
    ).subscribe((res) => {
      if (res?.version && res.version !== this.frontendVersion) {
        location.reload();
      }
    });
  }

  reconnectTimeout: any;
  intervalPing: any;
  public initSocket() {
    this.w = new WebSocket(environment.websocketUrl);
    this.w.onopen = () => {
      // Send JWT as first message for authentication
      const token = getJwtToken();
      if (token && this.w && this.w.readyState === this.w.OPEN) {
        this.w.send(token);
      }
      if (this.reconnectTimeout) {
        clearTimeout(this.reconnectTimeout);
      }
    };

    this.w.onmessage = (msg: { data: string }) => {
      let { data } = msg;
      try {
        const parsed = JSON.parse(data);
        if (!parsed) {
          return;
        }

        // Handle auth response
        if (parsed.type === 'AUTH') {
          if (parsed.success) {
            console.log('WebSocket authenticated');
          } else {
            console.error('WebSocket auth failed');
            this.w?.close();
          }
          return;
        }

        // Handle auth error (no type field)
        if (parsed.error) {
          console.error('WebSocket error:', parsed.error);
          this.w?.close();
          return;
        }

        if (!parsed.data || !parsed.type) {
          return;
        }
        switch (parsed.type) {
          case "WALLETS_FINANCIALS":
            this.watcherService.$walletsFinancialsInfo.next(parsed);
            break;
          case "LOGS":
            break;
          case "NEW_TX":
            this.watcherService.$newTx.next(parsed.data);
            break;
          case "UPDATE_TX":
            this.watcherService.$updateTx.next(parsed.data);
            break;
          case "TASK_STATUS":
            this.watcherService.$projectStatus.next(parsed.data);
            break;
          case "DAILY_VOLUME":
            this.watcherService.$dailyVolume.next(parsed.data);
            break;
          case "UPDATE_BALANCE":
            this.watcherService.$balanceUpdate.next(parsed.data);
            break;
          case "SOL_PRICE":
            this.watcherService.solPrice = parsed.data.ResponsePrice;
            break;
          case "ETH_PRICE":
            this.watcherService.ethPrice = parsed.data.ResponsePrice;
            break;
          case "SYSTEM_STATUS":
            this.systemStatus = parsed.data as SystemStatus;
            this.statusError = false;
            break;
        }
      } catch (err) {

      }
    };

    this.w.onclose = (ev: CloseEvent) => {
      console.log(`Websocket connection closed - reconnecting`);
      this.statusError = true;
      this.startAttemptingToEstablishConnection();
    }
  }

  private startAttemptingToEstablishConnection() {
    this.reconnectTimeout = setTimeout(() => {
      if (isLoggedIn()) {
        this.initSocket();
      } else {
        // Token expired — attempt refresh before reconnecting
        const refresh = getRefreshToken();
        if (refresh) {
          this.apiService.refreshToken(refresh).subscribe({
            next: (res) => {
              localStorage.setItem('vm_jwt', res.jwt);
              localStorage.setItem('vm_refresh_token', res.refresh_token);
              this.initSocket();
            },
            error: () => {
              this.helperService.logout();
            }
          });
        } else {
          this.helperService.logout();
        }
      }
    }, 5000);
  }
}
