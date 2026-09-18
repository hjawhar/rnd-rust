import { Injectable } from '@angular/core';
import { Subject } from 'rxjs';
import { Task } from '../models/task.model';

@Injectable({
    providedIn: 'root'
})
export class WatcherService {
    $updateTask = new Subject<{ id: number, data: Task }>();
    slotNumber: number | undefined;
}
