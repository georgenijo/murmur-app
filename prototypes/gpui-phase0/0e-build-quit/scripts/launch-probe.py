#!/usr/bin/env python3
import subprocess,os,json,pathlib,time,sys
root=pathlib.Path(__file__).resolve().parent.parent
label=sys.argv[1];bundle=root/'evidence/GPUI 0e Quit Probe.app';log=root/f'evidence/quit-{label}.log'
p=subprocess.Popen([str(bundle/'Contents/MacOS/gpui-0e-build-quit')],env={**os.environ,'P0E_LOG':str(log),'P0E_BUNDLE':str(bundle)},stdout=open(root/f'evidence/native-{label}-stdout.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
json.dump({'pid':p.pid,'start':time.time(),'log':str(log),'bundle':str(bundle)},open(root/f'evidence/native-{label}-process.json','w'));print(p.pid)
