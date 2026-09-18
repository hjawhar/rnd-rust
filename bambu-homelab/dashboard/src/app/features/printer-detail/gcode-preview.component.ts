import { Component, ElementRef, viewChild, signal, AfterViewInit, OnDestroy } from '@angular/core';

@Component({
  selector: 'app-gcode-preview',
  template: `
    <div class="bg-surface rounded-lg p-3">
      <h3 class="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">G-code Preview</h3>
      <div class="mb-3">
        <input type="file" (change)="onFileSelected($event)" accept=".gcode,.gco"
               class="text-sm text-text file:mr-2 file:py-1 file:px-3 file:rounded file:border-0 file:text-sm file:bg-surface-lighter file:text-text-muted hover:file:bg-primary hover:file:text-white" />
      </div>
      @if (loading()) {
        <div class="text-sm text-text-muted">Loading preview...</div>
      }
      @if (error()) {
        <div class="text-sm text-error">{{ error() }}</div>
      }
      <div class="w-full aspect-square rounded bg-black overflow-hidden" [class.hidden]="!loaded()">
        <canvas #previewCanvas class="w-full h-full block"></canvas>
      </div>
      @if (loaded()) {
        <div class="flex gap-2 mt-2 items-center">
          <input type="range" [min]="0" [max]="totalLayers()" [value]="currentLayer()"
                 (input)="onLayerChange($event)"
                 class="flex-1 accent-primary" />
          <span class="text-xs text-text-muted whitespace-nowrap">
            Layer {{ currentLayer() }} / {{ totalLayers() }}
          </span>
        </div>
      }
    </div>
  `,
})
export class GcodePreviewComponent implements AfterViewInit, OnDestroy {
  previewCanvas = viewChild<ElementRef<HTMLCanvasElement>>('previewCanvas');

  loading = signal(false);
  loaded = signal(false);
  error = signal<string | null>(null);
  totalLayers = signal(0);
  currentLayer = signal(0);

  private preview: any = null;
  private resizeObserver: ResizeObserver | null = null;

  ngAfterViewInit() {
    const canvas = this.previewCanvas()?.nativeElement;
    if (!canvas) return;

    // Keep canvas resolution in sync with its CSS size
    this.resizeObserver = new ResizeObserver(() => {
      if (!this.preview) return;
      const rect = canvas.getBoundingClientRect();
      canvas.width = rect.width * devicePixelRatio;
      canvas.height = rect.height * devicePixelRatio;
      this.preview.resize();
      this.preview.render();
    });
    this.resizeObserver.observe(canvas);
  }

  ngOnDestroy() {
    this.resizeObserver?.disconnect();
    this.preview?.dispose?.();
  }

  async onFileSelected(event: Event) {
    const file = (event.target as HTMLInputElement).files?.[0];
    if (!file) return;

    this.loading.set(true);
    this.loaded.set(false);
    this.error.set(null);

    try {
      const text = await file.text();
      const canvas = this.previewCanvas()?.nativeElement;
      if (!canvas) return;

      const { WebGLPreview } = await import('gcode-preview');

      this.preview?.dispose?.();

      // Set canvas to a usable resolution before initializing
      const rect = canvas.getBoundingClientRect();
      canvas.width = rect.width * devicePixelRatio;
      canvas.height = rect.height * devicePixelRatio;

      this.preview = new WebGLPreview({
        canvas,
        topLayerColor: '#3b82f6',
        lastSegmentColor: '#ef4444',
        buildVolume: { x: 256, y: 256, z: 256 },
        backgroundColor: '#1e1e2e',
      });

      this.preview.processGCode(text);

      const layerCount = this.preview.layers?.length ?? 0;
      this.totalLayers.set(layerCount);
      this.currentLayer.set(layerCount);
      this.loaded.set(true);
    } catch (e: any) {
      this.error.set(`Failed to load preview: ${e.message || e}`);
    } finally {
      this.loading.set(false);
    }
  }

  onLayerChange(event: Event) {
    const value = parseInt((event.target as HTMLInputElement).value, 10);
    this.currentLayer.set(value);
    this.preview?.renderLayer?.(value);
  }
}
