import { OnInit, OnDestroy, Injectable } from '@angular/core';
import { Subject } from 'rxjs';

@Injectable()
export class BasePageComponent implements OnInit, OnDestroy {
    componentDestroyed$: Subject<boolean> = new Subject();

    constructor() {
    }

    ngOnInit(): void {
    }

    ngOnDestroy(): void {
        this.componentDestroyed$.next(true);
        this.componentDestroyed$.complete();
    }
}