import { AfterViewInit, Component } from '@angular/core';
import { LocalSendService } from '../../shared/services/localsend.service';

@Component({
  selector: 'app-receive',
  standalone: true,
  imports: [],
  templateUrl: './receive.component.html',
  styleUrl: './receive.component.css'
})
export class ReceiveComponent implements AfterViewInit {
  constructor(public localSendService: LocalSendService) {
  }

  ngAfterViewInit(): void {
  }
}
