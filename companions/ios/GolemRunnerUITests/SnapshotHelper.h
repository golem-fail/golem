#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>

NS_ASSUME_NONNULL_BEGIN

/// Access private XCUIElementSnapshot properties safely from Objective-C.
@interface SnapshotHelper : NSObject

/// Returns the visibleFrame of a snapshot, or CGRectNull if unavailable.
+ (CGRect)visibleFrameOf:(id)snapshot;

@end

NS_ASSUME_NONNULL_END
