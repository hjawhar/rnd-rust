import { AfterViewInit, Component } from '@angular/core';
import { CommonModule } from '@angular/common';
import { RouterOutlet } from '@angular/router';
import { invoke } from "@tauri-apps/api/core";
import { SidePanelComponent } from './modules/side-panel/side-panel.component';
import { LocalSendService } from './shared/services/localsend.service';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [CommonModule, RouterOutlet, SidePanelComponent],
  providers: [LocalSendService],
  templateUrl: './app.component.html',
  styleUrl: './app.component.css'
})
export class AppComponent implements AfterViewInit {
  constructor(private localSendService: LocalSendService) {
  }

  ngAfterViewInit(): void {
  }
}
