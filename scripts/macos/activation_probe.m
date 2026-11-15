#import <AppKit/AppKit.h>
#import <UserNotifications/UserNotifications.h>
#include <stdio.h>
#include <fcntl.h>
#ifndef PROBE_LOG
#error PROBE_LOG must name the private test log
#endif
#include <unistd.h>
static void record(NSString *text) {
    int fd = open(PROBE_LOG, O_CREAT | O_APPEND | O_WRONLY | O_NOFOLLOW, 0600);
    if (fd < 0) { perror("activation probe log"); return; }
    if (dprintf(fd, "%d %s\n", getpid(), text.UTF8String) < 0) perror("activation probe write");
    if (close(fd) != 0) perror("activation probe close");
}
@interface Probe : NSObject <NSApplicationDelegate, UNUserNotificationCenterDelegate>
@end
@implementation Probe
- (void)applicationWillFinishLaunching:(NSNotification *)n {
    record([NSString stringWithFormat:@"willFinish %@", n.userInfo.allKeys]);
}
- (void)applicationDidFinishLaunching:(NSNotification *)n {
    record([NSString stringWithFormat:@"didFinish keys=%@ default=%@ notificationClass=%@", n.userInfo.allKeys, n.userInfo[NSApplicationLaunchIsDefaultLaunchKey], NSStringFromClass([n.userInfo[NSApplicationLaunchUserNotificationKey] class])]);
    NSAppleEventDescriptor *event = NSAppleEventManager.sharedAppleEventManager.currentAppleEvent;
    record([NSString stringWithFormat:@"appleEvent class=%u id=%u", event.eventClass, event.eventID]);
    if ([NSProcessInfo.processInfo.arguments containsObject:@"--notify"]) {
        UNUserNotificationCenter *c = UNUserNotificationCenter.currentNotificationCenter;
        [c requestAuthorizationWithOptions:UNAuthorizationOptionAlert completionHandler:^(BOOL yes, NSError *e) {
            record([NSString stringWithFormat:@"permission %d %@", yes, e]);
            if (!yes || e) return;
            UNMutableNotificationContent *content = [UNMutableNotificationContent new];
            content.title = @"Litty activation test";
            content.body = @"Click to verify notification routing.";
            content.userInfo = @{@"origin": @"probe", @"pane": @"1"};
            UNNotificationRequest *r = [UNNotificationRequest requestWithIdentifier:@"activation-probe" content:content trigger:nil];
            [c addNotificationRequest:r withCompletionHandler:^(NSError *error) { record([NSString stringWithFormat:@"delivery %@", error]); }];
        }];
    }
}
- (BOOL)applicationShouldHandleReopen:(NSApplication *)a hasVisibleWindows:(BOOL)b {
    record(@"reopen"); return NO;
}
- (void)applicationDidBecomeActive:(NSNotification *)n { record(@"becameActive"); }
- (void)userNotificationCenter:(UNUserNotificationCenter *)c didReceiveNotificationResponse:(UNNotificationResponse *)r withCompletionHandler:(void (^)(void))done {
    record([NSString stringWithFormat:@"response %@", r.actionIdentifier]); done();
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)c willPresentNotification:(UNNotification *)n withCompletionHandler:(void (^)(UNNotificationPresentationOptions))done {
    record(@"willPresent"); done(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionList);
}
@end
int main(void) {
    @autoreleasepool {
        record(@"main");
        NSApplication *app = NSApplication.sharedApplication;
        Probe *delegate = [Probe new];
        app.delegate = delegate;
        UNUserNotificationCenter.currentNotificationCenter.delegate = delegate;
        [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
        [app run];
    }
}
