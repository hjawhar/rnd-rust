import { Injectable } from '@angular/core';
import { Subject, Observable } from 'rxjs';

@Injectable({
  providedIn: 'root'
})
export class ProgressiveLoaderService {
  subject: Subject<boolean> = new Subject<boolean>();
  public constructor() {

  }
  public updateLoader(display: boolean) {
    this.subject.next(display);
  }

  public getState(): Observable<boolean> {
    return this.subject;
  }
}
