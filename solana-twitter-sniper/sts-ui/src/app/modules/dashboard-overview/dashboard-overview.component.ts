import { AfterViewInit, Component, ViewChild } from '@angular/core';
import { SharedModule } from '../../shared/shared.module';
import { CommonModule } from '@angular/common';
import { ApiService } from '../../shared/services/api.service';
import { environment } from '../../../environments/environment';
import { addressAbrev, getJwtToken } from '../../shared/utils';
import { TaskLogs, TxStatus } from '../../shared/models/solana_reqs.model';
import { MatSnackBar } from '@angular/material/snack-bar';
import { LogsType } from '../../shared/models/logs.enum';
import { RouterModule, RouterOutlet } from '@angular/router';
import { TwitterTweet } from '../../shared/models/twitter.model';
import { WatcherService } from '../../shared/services/watcher.service';
import { BasePageComponent } from '../../core/base-page.component';
import { Task } from '../../shared/models/task.model';

@Component({
  selector: 'app-dashboard-overview',
  imports: [CommonModule, RouterOutlet, RouterModule, SharedModule],
  templateUrl: './dashboard-overview.component.html',
  styleUrl: './dashboard-overview.component.scss'
})
export class DashboardOverviewComponent extends BasePageComponent implements AfterViewInit {
  LogsType = LogsType;
  w?: WebSocket;
  @ViewChild('twitterAudio') twitterAudio: any;
  @ViewChild('confirmedAudio') confirmedAudio: any;
  @ViewChild('sendingAudio') sendingAudio: any;

  addressAbrev = addressAbrev;

  tasks: Task[] = [];
  tweets: { server: string, tweet: { tweet: TwitterTweet, timestamp: number, slot: number } }[] = [];
  logs: { server: string, logs: TaskLogs }[] = [];

  constructor(private apiService: ApiService, private snackbar: MatSnackBar, private watcherService: WatcherService) {
    super();
  }

  ngAfterViewInit(): void {
    this.initSocket();

    this.watcherService.$updateTask.subscribe(response => {
      if (response.id && response['data']) {
        let idx = this.tasks.findIndex(x => x.id === response.id);
        if (idx >= 0) {
          this.tasks[idx] = response['data'];
        }
      }
    })
  }

  public clearTweets() {
    this.tweets = [];
  }

  public clearLogs() {
    this.logs = [];
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
            case "TWEET":
              {
                this.twitterAudio.nativeElement.play();
                this.tweets = [{ server: parsed.server, tweet: parsed.data }, ...this.tweets];
              }
              break;
            case "LOGS":
              let tx = parsed.data as TaskLogs;
              let logs_type = tx.logs_type;
              if (logs_type == LogsType.BUY) {
                if (tx.status == TxStatus.SENT) {
                  this.sendingAudio.nativeElement.play();
                } else if (tx.status == TxStatus.CONFIRMED) {
                  this.confirmedAudio.nativeElement.play();
                }
                this.logs = [{ server: parsed.server, logs: tx }, ...this.logs];
              } else if (logs_type == LogsType.IMAGE) {
                this.logs = [{ server: parsed.server, logs: tx }, ...this.logs];
              } else {
              }
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
