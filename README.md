# tokio-task-example

Demonstrates task polling for tokio

I didn't really understand how futures/tasks are scheduled, so I put together a small test
program to illustrate the advantages of task-aware future polling.  This program schedules
a million futures in a timer, each of which will complete within a minute.

One of two compile-time-configurable methods is used to process pending futures.

The _iterating_ approach naively tracks a VecDeque of pending futures, iterating over all
futures on each poll:
```
$ ./target/release/tokio-task-example
iterating events=1000000 max=60000ms granularity=10ms
poll time 51.370256144
./target/release/tokio-task-example  50.53s user 0.50s system 84% cpu 1:00.13 total
```

The _unparking_ approach, which uses the `futures::task` API to only poll relevant pending
futures is substantially more efficient CPU-wise:
```
unparking events=1000000 max=60000ms granularity=10ms
poll time 11.91080895
./target/release/tokio-task-example  4.76s user 9.65s system 23% cpu 1:00.14 totall
```
