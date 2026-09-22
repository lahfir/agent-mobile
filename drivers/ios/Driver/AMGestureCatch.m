#import "AMGestureCatch.h"

@implementation AMGestureCatch
+ (nullable NSString *)runGesture:(void (^)(void))block {
    @try {
        block();
        return nil;
    } @catch (NSException *e) {
        return [NSString stringWithFormat:@"%@: %@", e.name, e.reason];
    }
}
@end
