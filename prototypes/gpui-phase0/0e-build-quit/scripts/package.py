#!/usr/bin/env python3
import os,pathlib,plistlib,shutil,subprocess,sys
root=pathlib.Path(__file__).resolve().parent.parent
binary=pathlib.Path(sys.argv[1]).resolve()
bundle=root/'evidence/GPUI 0e Quit Probe.app'
(bundle/'Contents/MacOS').mkdir(parents=True,exist_ok=True)
shutil.copy2(binary,bundle/'Contents/MacOS/gpui-0e-build-quit')
with (bundle/'Contents/Info.plist').open('wb') as f:
    plistlib.dump(dict(CFBundleIdentifier='com.georgenijo.gpui0e',CFBundleName='GPUI 0e Quit Probe',CFBundleExecutable='gpui-0e-build-quit',CFBundlePackageType='APPL',CFBundleVersion='1',NSHighResolutionCapable=True,LSMinimumSystemVersion='12.0'),f)
subprocess.run(['codesign','--force','--sign','-',str(bundle)],check=True)
print(bundle)
