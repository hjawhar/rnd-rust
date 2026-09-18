import { Component, input, computed, inject, signal, ElementRef, viewChild, effect, OnDestroy } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import Hls from 'hls.js';

@Component({
  selector: 'app-camera-panel',
  template: `
    <div class="bg-surface-light rounded-lg border border-border p-3 h-full flex flex-col">
      @if (rtspUrl()) {
        <div class="flex-1 min-h-0">
          @if (streamError()) {
            <div class="w-full h-full min-h-[200px] rounded bg-surface flex flex-col items-center justify-center">
              <span class="text-error text-sm">Stream unavailable</span>
              <span class="text-text-muted text-xs mt-1">Start video services: docker compose up -d video-relay hls</span>
              <button (click)="streamError.set(false); showStream.set(false)" class="text-xs text-primary mt-2 hover:text-primary-light">Dismiss</button>
            </div>
          } @else if (showStream()) {
            <video #videoPlayer autoplay muted playsinline
                   class="w-full h-full rounded bg-black object-contain">
            </video>
          } @else if (loadingStream()) {
            <div class="w-full h-full min-h-[200px] rounded bg-surface flex items-center justify-center">
              <span class="text-text-muted animate-pulse">Loading camera stream...</span>
            </div>
          } @else {
            <button (click)="loadStream()"
                    class="w-full h-full min-h-[200px] rounded bg-surface flex items-center justify-center hover:bg-surface-lighter transition-colors">
              <span class="text-text-muted">Click to load camera stream</span>
            </button>
          }
        </div>

        @if (!readOnly()) {
          <div class="flex items-center gap-2 mt-2 shrink-0">
            <button (click)="startTimelapse()" [disabled]="timelapseActive()"
                    class="px-2 py-1 bg-primary/20 text-primary rounded text-[10px] font-medium disabled:opacity-40">
              Start Timelapse
            </button>
            <button (click)="stopTimelapse()" [disabled]="!timelapseActive()"
                    class="px-2 py-1 bg-error/20 text-error rounded text-[10px] font-medium disabled:opacity-40">
              Stop &amp; Save
            </button>
            @if (timelapseUrl()) {
              <a [href]="timelapseUrl()" target="_blank"
                 class="px-2 py-1 bg-success/20 text-success rounded text-[10px] font-medium">
                View Timelapse
              </a>
            }
            @if (showStream()) {
              <button (click)="hideStream()" class="text-[10px] text-text-muted hover:text-error">Hide</button>
            }
            <span class="text-[10px] text-text-muted truncate ml-auto">{{ rtspUrl() }}</span>
          </div>
        }
      } @else {
        <div class="text-xs text-text-muted">No camera stream available</div>
      }
    </div>
  `,
})
export class CameraPanelComponent implements OnDestroy {
  rtspUrl = input<string>('');
  printerId = input<string>('');
  readOnly = input(false);

  private http = inject(HttpClient);
  private hls: Hls | null = null;

  timelapseActive = signal(false);
  showStream = signal(false);
  streamError = signal(false);
  loadingStream = signal(false);

  videoPlayer = viewChild<ElementRef<HTMLVideoElement>>('videoPlayer');

  hlsUrl = computed(() => {
    const id = this.printerId();
    return id ? `/streams/${id}/stream.m3u8` : '';
  });

  timelapseUrl = computed(() => {
    const id = this.printerId();
    return id ? `/timelapses/${id}/timelapse.mp4` : '';
  });

  constructor() {
    // When showStream becomes true and the video element is available, attach HLS.js
    effect(() => {
      if (this.showStream()) {
        // Wait for Angular to render the video element
        setTimeout(() => this.attachHls(), 0);
      }
    });
  }

  ngOnDestroy() {
    this.destroyHls();
  }

  private attachHls() {
    const video = this.videoPlayer()?.nativeElement;
    if (!video) return;

    const url = this.hlsUrl();
    if (!url) return;

    this.destroyHls();

    if (Hls.isSupported()) {
      this.hls = new Hls({
        enableWorker: true,
        lowLatencyMode: true,
        liveSyncDurationCount: 2,
      });
      this.hls.loadSource(url);
      this.hls.attachMedia(video);
      this.hls.on(Hls.Events.ERROR, (_event, data) => {
        if (data.fatal) {
          this.streamError.set(true);
          this.showStream.set(false);
          this.destroyHls();
        }
      });
    } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
      // Safari native HLS
      video.src = url;
    } else {
      this.streamError.set(true);
      this.showStream.set(false);
    }
  }

  private destroyHls() {
    if (this.hls) {
      this.hls.destroy();
      this.hls = null;
    }
  }

  hideStream() {
    this.showStream.set(false);
    this.destroyHls();
  }

  loadStream() {
    this.loadingStream.set(true);
    this.streamError.set(false);
    this.http.post(`/api/printers/${this.printerId()}/stream/start`, {}).subscribe({
      next: () => {
        setTimeout(() => {
          this.loadingStream.set(false);
          this.showStream.set(true);
        }, 2000);
      },
      error: () => {
        this.loadingStream.set(false);
        this.streamError.set(true);
      },
    });
  }

  startTimelapse() {
    this.http.post(`/api/printers/${this.printerId()}/timelapse/start`, {}).subscribe({
      next: () => this.timelapseActive.set(true),
    });
  }

  stopTimelapse() {
    this.http.post(`/api/printers/${this.printerId()}/timelapse/stop`, {}).subscribe({
      next: () => this.timelapseActive.set(false),
    });
  }
}
