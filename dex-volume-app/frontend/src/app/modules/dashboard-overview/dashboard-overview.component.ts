import { AfterViewInit, Component, ViewChild } from '@angular/core';
import { SharedModule } from '../../shared/shared.module';
import { CommonModule } from '@angular/common';
import { ApiService } from '../../shared/services/api.service';
import { environment } from '../../../environments/environment';
import { addressAbrev, getJwtToken } from '../../shared/utils';
import { MatSnackBar } from '@angular/material/snack-bar';
import { RouterModule, RouterOutlet } from '@angular/router';
import { WatcherService } from '../../shared/services/watcher.service';
import { BasePageComponent } from '../../core/base-page.component';

@Component({
  selector: 'app-dashboard-overview',
  imports: [CommonModule, RouterOutlet, RouterModule, SharedModule],
  templateUrl: './dashboard-overview.component.html',
  styleUrl: './dashboard-overview.component.scss'
})
export class DashboardOverviewComponent extends BasePageComponent implements AfterViewInit {
  w?: WebSocket;
  @ViewChild('twitterAudio') twitterAudio: any;
  @ViewChild('confirmedAudio') confirmedAudio: any;
  @ViewChild('sendingAudio') sendingAudio: any;

  addressAbrev = addressAbrev;


  constructor(private apiService: ApiService, private snackbar: MatSnackBar, private watcherService: WatcherService) {
    super();
  }

  ngAfterViewInit(): void {
    this.initSocket();
  }

  reconnectTimeout: any;
  intervalPing: any;
  public initSocket() {
    this.w = new WebSocket(`${environment.websocketUrl}?token=${getJwtToken()}`);
    this.w.onopen = (e) => {
      console.log('Successfully connected');
      if (this.reconnectTimeout) {
        clearTimeout(this.reconnectTimeout);
      }

    };

    this.w.onmessage = (msg: { data: string }) => {
      let { data } = msg;
      try {
        if (data === 'PING') {
          if (this.w && this.w.readyState === this.w.OPEN) {
            this.w.send('PING');
          }
        } else {
          const parsed = JSON.parse(data);
          if (!parsed) {
            return;
          }
          if (!parsed.data || !parsed.type) {
            return;
          }
          switch (parsed.type) {
            case "LOGS":

              break;
            case "SLOT_NUMBER":
              if (parsed.server === "DE") {
                this.watcherService.slotNumber = parsed.data;
              }
              break;
          }
        }
      } catch (err) {

      }
    };

    this.w.onclose = (ev: CloseEvent) => {
      console.log(`Websocket connection closed - reconnecting`);
      this.startAttemptingToEstablishConnection();
    }
  }

  private startAttemptingToEstablishConnection() {
    this.reconnectTimeout = setTimeout(() => this.initSocket(), 5000);
  }
}
