#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>

NS_ASSUME_NONNULL_BEGIN

/// Access private XCUIElementSnapshot properties safely from Objective-C.
@interface SnapshotHelper : NSObject

/// Returns the visibleFrame of a snapshot, or CGRectNull if unavailable.
+ (CGRect)visibleFrameOf:(id)snapshot;

/// Run `block` inside an Obj-C `@try/@catch`. Swift can't catch
/// NSException via `try`; when XCUITest internals raise (XCTWaiter
/// timeout, missing bundle, etc.) the exception otherwise unwinds
/// to `_XCTTerminateHandler` and SIGABRTs the harness. Use this to
/// keep handlers isolated so one bad request can't kill the runner.
/// Returns YES on success, NO if block raised (outException set).
+ (BOOL)catchNSException:(void (^)(void))block
         exception:(NSException * _Nullable * _Nullable)outException;

@end

NS_ASSUME_NONNULL_END
