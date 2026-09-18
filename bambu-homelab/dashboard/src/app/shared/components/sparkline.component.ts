import { Component, input, computed, ElementRef, viewChild, effect } from '@angular/core';

@Component({
  selector: 'app-sparkline',
  template: `<canvas #canvas [width]="width()" [height]="height()" class="rounded"></canvas>`,
})
export class SparklineComponent {
  data = input.required<number[]>();
  color = input('var(--color-primary)');
  width = input(200);
  height = input(40);

  canvas = viewChild.required<ElementRef<HTMLCanvasElement>>('canvas');

  constructor() {
    effect(() => {
      const points = this.data();
      const el = this.canvas();
      if (!el || points.length < 2) return;

      const ctx = el.nativeElement.getContext('2d');
      if (!ctx) return;

      const w = el.nativeElement.width;
      const h = el.nativeElement.height;
      const min = Math.min(...points);
      const max = Math.max(...points);
      const range = max - min || 1;

      ctx.clearRect(0, 0, w, h);
      ctx.strokeStyle = this.color();
      ctx.lineWidth = 1.5;
      ctx.beginPath();

      points.forEach((val, i) => {
        const x = (i / (points.length - 1)) * w;
        const y = h - ((val - min) / range) * (h - 4) - 2;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });

      ctx.stroke();
    });
  }
}
