import { CommonModule } from '@angular/common';
import { Component } from '@angular/core';
import { RouterModule } from '@angular/router';
import { invoke } from '@tauri-apps/api/core';

@Component({
  selector: 'app-side-panel',
  standalone: true,
  imports: [
    CommonModule,
    RouterModule
  ],
  templateUrl: './side-panel.component.html',
  styleUrl: './side-panel.component.css'
})
export class SidePanelComponent {
 
}
