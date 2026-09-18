import { Injectable, inject, signal } from '@angular/core';
import { webSocket, WebSocketSubject } from 'rxjs/webSocket';
import { Observable, retry, timer, share, filter } from 'rxjs';
import {
  WsServerMessage,
  WsClientMessage,
  WsAssignmentAddedMessage,
  WsAssignmentRemovedMessage,
} from '../models/command.model';
import { AuthService } from './auth.service';

@Injectable({ providedIn: 'root' })
export class WebSocketService {
  private auth = inject(AuthService);
  private socket$: WebSocketSubject<WsServerMessage | WsClientMessage> | null = null;

  readonly connected = signal(false);

  private messages$ = this.createSocket();

  readonly telemetry$ = this.messages$.pipe(
    filter((msg): msg is Extract<WsServerMessage, { type: 'telemetry' }> =>
      'type' in msg && msg.type === 'telemetry'
    ),
  );

  readonly assignmentAdded$ = this.messages$.pipe(
    filter((msg): msg is WsAssignmentAddedMessage =>
      'type' in msg && msg.type === 'assignment_added'
    ),
  );

  readonly assignmentRemoved$ = this.messages$.pipe(
    filter((msg): msg is WsAssignmentRemovedMessage =>
      'type' in msg && msg.type === 'assignment_removed'
    ),
  );

  private createSocket(): Observable<WsServerMessage> {
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    this.socket$ = webSocket<WsServerMessage | WsClientMessage>({
      url: `${protocol}//${location.host}/api/ws`,
      openObserver: {
        next: () => {
          this.connected.set(true);
          // Send auth message immediately on connect
          const token = this.auth.token();
          if (token) {
            this.socket$?.next({ type: 'auth', token });
          }
        },
      },
      closeObserver: {
        next: () => this.connected.set(false),
      },
    });

    return (this.socket$ as Observable<WsServerMessage>).pipe(
      retry({ delay: (_err, retryCount) => timer(Math.min(1000 * 2 ** retryCount, 30000)) }),
      share(),
    );
  }

  subscribe(printerIds: string[]): void {
    this.socket$?.next({ type: 'subscribe', printer_ids: printerIds });
  }

  unsubscribe(printerIds: string[]): void {
    this.socket$?.next({ type: 'unsubscribe', printer_ids: printerIds });
  }
}
