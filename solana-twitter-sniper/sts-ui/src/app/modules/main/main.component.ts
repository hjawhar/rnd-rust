import { CommonModule } from '@angular/common';
import { Component } from '@angular/core';
import { SharedModule } from '../../shared/shared.module';
import { RouterModule } from '@angular/router';
import { WatcherService } from '../../shared/services/watcher.service';
import { HelperService } from '../../shared/services/helper.service';
import { addressAbrev } from '../../shared/utils';

@Component({
  selector: 'app-main',
  templateUrl: './main.component.html',
  styleUrl: './main.component.scss',
  imports: [CommonModule, SharedModule, RouterModule]
})
export class MainComponent {
  addressAbrev = addressAbrev;
  constructor(public watcherService: WatcherService, public helperService: HelperService) {
  }
}
