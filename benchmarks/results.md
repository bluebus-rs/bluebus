| | bluebus | bluer | btleplug | bleak (Python) | bluebus fd | bluer fd | bleak (Python) fd | bluebus (Python) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Startup (session + adapter ready) | 3.4 ms | 3.3 ms | 3.5 ms | 30.1 ms | 3.9 ms | 2.7 ms | 29.1 ms | 31.7 ms |
| Connect + GATT services resolved | 26.0 ms | 30.1 ms | 30.8 ms | 36.5 ms | 24.2 ms | 30.5 ms | 28.6 ms | 23.9 ms |
| Read, p50 | 0.280 ms | 0.293 ms | 0.278 ms | 0.301 ms | 0.309 ms | 0.287 ms | 0.299 ms | 0.420 ms |
| Read, p95 | 0.866 ms | 0.839 ms | 0.698 ms | 0.885 ms | 1.005 ms | 0.905 ms | 0.956 ms | 1.085 ms |
| Write with response, p50 | 0.105 ms | 0.105 ms | 0.107 ms | 0.121 ms | 0.111 ms | 0.107 ms | 0.137 ms | 0.164 ms |
| Write with response, p95 | 0.586 ms | 0.452 ms | 0.670 ms | 0.583 ms | 0.527 ms | 0.430 ms | 0.721 ms | 0.829 ms |
| 5000 notifications: received | 5000 | 5000 | 5000 | 5001 | 5000 | 5000 | 5000 | 5000 |
| 5000 notifications: time | 424.1 ms | 414.4 ms | 428.5 ms | 419.2 ms | 95.7 ms | 96.6 ms | 86.0 ms | 432.7 ms |
| CPU time: reads + writes | 28 ms | 31 ms | 32 ms | 46 ms | 29 ms | 34 ms | 52 ms | 93 ms |
| CPU time: 5000 notifications | 66 ms | 58 ms | 63 ms | 35 ms | 7 ms | 8 ms | 13 ms | 243 ms |
| CPU time, whole process | 113 ms | 110 ms | 116 ms | 162 ms | 53 ms | 61 ms | 139 ms | 398 ms |
| Peak memory (RSS) | 5.4 MB | 5.9 MB | 7.7 MB | 27.9 MB | 5.4 MB | 5.8 MB | 27.9 MB | 29.1 MB |
