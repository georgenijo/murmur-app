#!/usr/bin/env python3
"""Native input -> receiver log -> UI snapshots -> ordering/exit assertions."""
import subprocess,time,pathlib,json,os,re,sys
root=pathlib.Path(__file__).resolve().parent.parent
os.chdir(root)
bundle=root/'evidence/GPUI 0e Quit Probe.app'
label=sys.argv[1]
action=label.split("-")[0]
log=root/f'evidence/quit-release-{label}.log'
process=subprocess.Popen([str(bundle/'Contents/MacOS/gpui-0e-build-quit')],env={**os.environ,'P0E_LOG':str(log),'P0E_BUNDLE':str(bundle)},stdout=open(root/f'evidence/release-{label}-stdout.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
pid=process.pid
json.dump({'pid':pid,'start':time.time(),'bundle':str(bundle)},open(root/f'evidence/release-{label}-process.json','w'))
def wait_for(predicate,timeout=12):
 start=time.monotonic()
 while time.monotonic()-start<timeout:
  if predicate():return
  time.sleep(.05)
 raise RuntimeError(f'{label}: timeout, pid={pid}')
def text():return log.read_text() if log.exists() else ''
def run(args,name):
 with open(root/f'evidence/release-{label}-{name}.txt','w') as f:
  p=subprocess.run(args,stdout=f,stderr=subprocess.STDOUT)
  if p.returncode:raise RuntimeError(f'{args}: exit {p.returncode}')
wait_for(lambda:' READY' in text())
if action=='menu':run(['ghosthands','menu','GPUI 0e > Quit GPUI 0e',str(pid)],'action')
elif action=='key':run(['ghosthands','key','cmd+q',str(pid),'--visible'],'action')
elif action=='logout':run([str(root/'evidence/send-logout-quit'),str(pid)],'action')
elif action=='relaunch':run(['ghosthands','menu','GPUI 0e > Relaunch Probe',str(pid)],'action')
else:raise ValueError(label)
wait_for(lambda:'TEARDOWN_START' in text(),5)
for i in range(2):
 time.sleep(1)
 run(['ghosthands','snapshot',str(pid),'--ax'],f'ax-during-{i}')
run(['ghosthands','shot',str(pid),str(root/f'evidence/release-{label}-during.png')],'shot')
wait_for(lambda:f'pid={pid} main=1 GPUI_QUIT_HANDLER_END' in text())
wait_for(lambda:process.poll() is not None,5)
assert process.returncode == 0, process.returncode
if action=='relaunch':
 wait_for(lambda:len(re.findall(r'pid=(\d+) main=1 PROCESS_START',text()))>=2)
 child=int(re.findall(r'pid=(\d+) main=1 PROCESS_START',text())[-1]);assert child!=pid
 wait_for(lambda:f'pid={child} main=1 READY' in text())
 json.dump({'old_pid':pid,'new_pid':child},open(root/'evidence/release-relaunch-pids.json','w'))
 print(f'relaunch child={child}',flush=True)
print(f'{label}: quit and exit observed, pid={pid}',flush=True)
