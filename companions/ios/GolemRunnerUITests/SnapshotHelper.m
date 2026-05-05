#import "SnapshotHelper.h"
#import <objc/runtime.h>
#import <objc/message.h>

@implementation SnapshotHelper

+ (CGRect)visibleFrameOf:(id)snapshot {
    SEL sel = NSSelectorFromString(@"visibleFrame");
    if (![snapshot respondsToSelector:sel]) {
        return CGRectNull;
    }
    // visibleFrame returns a CGRect struct — use objc_msgSend_stret on arm64
    // or the typed function pointer approach to avoid ABI issues.
    typedef CGRect (*VisibleFrameIMP)(id, SEL);
    VisibleFrameIMP imp = (VisibleFrameIMP)[snapshot methodForSelector:sel];
    if (!imp) {
        return CGRectNull;
    }
    @try {
        return imp(snapshot, sel);
    } @catch (NSException *e) {
        return CGRectNull;
    }
}

+ (BOOL)catchNSException:(void (^)(void))block
         exception:(NSException * _Nullable * _Nullable)outException {
    @try {
        block();
        return YES;
    } @catch (NSException *e) {
        if (outException) {
            *outException = e;
        }
        return NO;
    }
}

@end
