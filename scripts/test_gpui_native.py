"""Compile the focus shim against inert classes; never create AppKit windows."""
from pathlib import Path
import platform
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(platform.system() == 'Darwin', 'Objective-C runtime requires macOS')
class FocusHookTests(unittest.TestCase):
    def test_effective_appearance_selects_light_dark_and_accessible_variants(self):
        source = r'''
#import <AppKit/AppKit.h>
#include <assert.h>
extern unsigned murmur_display_preferences(void);
int main(void) {
    @autoreleasepool {
        NSApplication *app = NSApplication.sharedApplication;
        unsigned preferences = murmur_display_preferences() & 7;
        for (NSString *name in @[NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
                NSAppearanceNameAccessibilityHighContrastAqua,
                NSAppearanceNameAccessibilityHighContrastDarkAqua, NSAppearanceNameAqua]) {
            // Process-local appearance only: no windows, activation, or settings changes.
            app.appearance = [NSAppearance appearanceNamed:name];
            unsigned bits = murmur_display_preferences();
            assert((bits & 7) == preferences);
            assert(((bits & 8) != 0) == [name containsString:@"Dark"]);
        }
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-appearance-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            str(ROOT / 'app/gpui/native/preferences.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)

    def test_window_and_panel_forward_focus_and_keep_fallbacks(self):
        source = r'''
#import <Foundation/Foundation.h>
#import <objc/runtime.h>
#include <assert.h>
@interface Content : NSObject
@end
@implementation Content
- (id)accessibilityFocusedUIElement { return self; }
@end
@interface OriginalWindow : NSObject
@property id contentView;
@end
@implementation OriginalWindow
- (NSArray *)accessibilityAttributeNames { return @[@"AXTitle"]; }
- (id)accessibilityAttributeValue:(NSString *)attribute {
    return [attribute isEqualToString:@"AXTitle"] ? @"synthetic window" : nil;
}
@end
@interface GPUIWindow : OriginalWindow
@end
@implementation GPUIWindow
@end
@interface GPUIPanel : OriginalWindow
@end
@implementation GPUIPanel
@end
@interface OriginalApplication : NSObject
@property id keyWindow;
@end
@implementation OriginalApplication
- (id)accessibilityFocusedUIElement { return self; }
@end
@interface GPUIApplication : OriginalApplication
@end
@implementation GPUIApplication
@end
extern void murmur_install_focus_hook(void);
int main(void) {
    @autoreleasepool {
        murmur_install_focus_hook();
        murmur_install_focus_hook();
        id content = [Content new];
        for (Class cls in @[[GPUIWindow class], [GPUIPanel class]]) {
            id window = [cls new];
            assert([window respondsToSelector:@selector(accessibilityFocusedUIElement)]);
            assert([window accessibilityFocusedUIElement] == window);
            assert([[window accessibilityAttributeNames] containsObject:@"AXFocusedUIElement"]);
            assert([[window accessibilityAttributeNames] containsObject:@"AXTitle"]);
            assert([[window accessibilityAttributeValue:@"AXTitle"] isEqualToString:@"synthetic window"]);
            assert([window accessibilityAttributeValue:@"AXFocusedUIElement"] == window);
            [window setContentView:content];
            assert([window accessibilityFocusedUIElement] == content);
            assert([window accessibilityAttributeValue:@"AXFocusedUIElement"] == content);
        }
        id app = [GPUIApplication new];
        assert([app accessibilityFocusedUIElement] == app);
        id panel = [GPUIPanel new];
        [panel setContentView:content];
        [app setKeyWindow:panel];
        assert([app accessibilityFocusedUIElement] == content);
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-focus-hook-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            str(ROOT / 'app/gpui/native/ax_focus_fix.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)


if __name__ == '__main__':
    unittest.main()
