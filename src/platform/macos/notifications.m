#import <AppKit/AppKit.h>
#import <UserNotifications/UserNotifications.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <libproc.h>
#include <string.h>

typedef void (*LittyNotificationCallback)(const char *, uint64_t);
typedef void (*LittyNotificationFallback)(const char *, const char *);
static LittyNotificationCallback route;
// Shows a notification without Litty's identity when it is not authorized.
static LittyNotificationFallback fallback;
static atomic_bool cold_start;
static atomic_uint pending;
static uint64_t sequence;
static id launch_observer;
static BOOL available;
static atomic_bool shutting_down;
static NSMutableOrderedSet<NSString *> *owned_identifiers;

// Shared by launch and live callbacks; stale Pane activations are idempotent.
@interface LittyNotificationDelegate : NSObject <UNUserNotificationCenterDelegate>
@property(nonatomic) NSMutableOrderedSet<NSString *> *seen;
@property(nonatomic) BOOL closed;
@property(nonatomic, copy) void (^routeAction)(NSString *, uint64_t);
- (void)receiveIdentifier:(NSString *)identifier action:(NSString *)action origin:(NSString *)origin pane:(NSString *)pane;
- (void)handleIdentifier:(NSString *)identifier action:(NSString *)action origin:(NSString *)origin pane:(NSString *)pane completion:(void (^)(void))done;
- (void)receive:(UNNotificationResponse *)response;
@end
static LittyNotificationDelegate *notification_delegate;

static BOOL valid_origin(NSString *value) {
    if (![value isKindOfClass:NSString.class] || [value lengthOfBytesUsingEncoding:NSUTF8StringEncoding] > 58) return NO;
    NSArray<NSString *> *pieces = [value componentsSeparatedByString:@"-"];
    if (pieces.count != 3 || pieces[0].length != 16 || pieces[1].length == 0 || pieces[1].length > 8 || pieces[2].length != 32) return NO;
    NSCharacterSet *bad = [[NSCharacterSet characterSetWithCharactersInString:@"0123456789abcdefABCDEF"] invertedSet];
    for (NSString *piece in pieces) if ([piece rangeOfCharacterFromSet:bad].location != NSNotFound) return NO;
    return YES;
}

@implementation LittyNotificationDelegate
- (void)receive:(UNNotificationResponse *)response {
    NSDictionary *info = response.notification.request.content.userInfo;
    [self receiveIdentifier:response.notification.request.identifier action:response.actionIdentifier origin:info[@"origin"] pane:info[@"pane"]];
}
- (void)receiveIdentifier:(NSString *)identifier action:(NSString *)action origin:(NSString *)origin pane:(NSString *)pane {
    if (self.closed || ![action isEqual:UNNotificationDefaultActionIdentifier]) return;
    if (![identifier isKindOfClass:NSString.class] || identifier.length > 160 || ![identifier hasPrefix:@"litty-pane-"] || [self.seen containsObject:identifier]) return;
    if (!valid_origin(origin) || ![pane isKindOfClass:NSString.class] || pane.length == 0 || pane.length > 20) return;
    uint64_t pane_id = 0;
    for (NSUInteger i = 0; i < pane.length; i++) {
        unichar ch = [pane characterAtIndex:i];
        if (ch < '0' || ch > '9' || pane_id > (UINT64_MAX - (ch - '0')) / 10) return;
        pane_id = pane_id * 10 + (ch - '0');
    }
    if (!pane_id) return;
    [self.seen addObject:identifier];
    if (self.seen.count > 64) [self.seen removeObjectAtIndex:0];
    if (self.routeAction) self.routeAction(origin, pane_id);
}
- (void)handleIdentifier:(NSString *)identifier action:(NSString *)action origin:(NSString *)origin pane:(NSString *)pane completion:(void (^)(void))done {
    @try { [self receiveIdentifier:identifier action:action origin:origin pane:pane]; }
    @finally { done(); }
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)center didReceiveNotificationResponse:(UNNotificationResponse *)response withCompletionHandler:(void (^)(void))done {
    (void)center;
    NSDictionary *info = response.notification.request.content.userInfo;
    void (^process)(void) = ^{
        [self handleIdentifier:response.notification.request.identifier action:response.actionIdentifier origin:info[@"origin"] pane:info[@"pane"] completion:done];
    };
    // Finish routing before telling macOS it may deliver activation/reopen.
    if (NSThread.isMainThread) process();
    else dispatch_sync(dispatch_get_main_queue(), process);
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)center willPresentNotification:(UNNotification *)notification withCompletionHandler:(void (^)(UNNotificationPresentationOptions))done {
    (void)center; (void)notification;
    done(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionList);
}
@end

bool litty_notifications_init(LittyNotificationCallback callback, LittyNotificationFallback unauthorized) {
    if (![NSThread isMainThread]) return false;
    route = callback;
    fallback = unauthorized;
    if (available) return true;
    NSBundle *bundle = NSBundle.mainBundle;
    if (![bundle.bundlePath.pathExtension isEqualToString:@"app"] || bundle.bundleIdentifier.length == 0) return false;
    owned_identifiers = [NSMutableOrderedSet new];
    notification_delegate = [LittyNotificationDelegate new];
    notification_delegate.seen = [NSMutableOrderedSet new];
    notification_delegate.routeAction = ^(NSString *origin, uint64_t pane) { if (route) route(origin.UTF8String, pane); };
    UNUserNotificationCenter.currentNotificationCenter.delegate = notification_delegate;
    launch_observer = [NSNotificationCenter.defaultCenter addObserverForName:NSApplicationDidFinishLaunchingNotification object:nil queue:nil usingBlock:^(NSNotification *note) {
        id response = note.userInfo[NSApplicationLaunchUserNotificationKey];
        if ([response isKindOfClass:UNNotificationResponse.class]) {
            atomic_store(&cold_start, true);
            [notification_delegate receive:response];
        }
    }];
    available = YES;
    return true;
}
bool litty_notification_startup(void) { return atomic_load(&cold_start); }
static BOOL notification_reopen_source(AEEventClass event_class, AEEventID event_id, const char *sender_path);
bool litty_notification_consume_reopen(void) {
    // Inspect synchronously: currentAppleEvent is valid only during dispatch.
    // Its sender PID is supplied by the OS, not by notification payload text.
    NSAppleEventDescriptor *event = NSAppleEventManager.sharedAppleEventManager.currentAppleEvent;
    if (!event || event.eventClass != kCoreEventClass || event.eventID != kAEReopenApplication) return false;
    pid_t sender = [event attributeDescriptorForKeyword:keySenderPIDAttr].int32Value;
    if (sender <= 0) return false;
    char path[PROC_PIDPATHINFO_MAXSIZE] = {0};
    if (proc_pidpath(sender, path, sizeof(path)) <= 0) return false;
    return notification_reopen_source(event.eventClass, event.eventID, path);
}

// A ticket is completed at most once by delivery, failure or deadline.
@interface LittyNotificationTicket : NSObject { @public atomic_bool finished; atomic_bool submission_started; }
@end
@implementation LittyNotificationTicket
@end
static BOOL finish(LittyNotificationTicket *ticket) {
    if (atomic_exchange(&ticket->finished, true)) return NO;
    return YES;
}
static BOOL expire_submission(LittyNotificationTicket *ticket) {
    return atomic_load(&ticket->submission_started) && finish(ticket);
}
// Denied or unavailable authorization completes the ticket once. The text
// still reaches the person through the identity-less helper (no click routing).
static BOOL handle_authorization_failure(LittyNotificationTicket *ticket, BOOL granted, NSError *error, NSString *title, NSString *body) {
    if (granted && !error) return NO;
    if (finish(ticket)) {
        fprintf(stderr, "litty: notification authorization unavailable; using helper\n");
        if (fallback) fallback(title.UTF8String, body.UTF8String);
    }
    return YES;
}
static NSUInteger test_fallbacks;
static BOOL test_fallback_exact;
static void count_test_fallback(const char *title, const char *body) {
    test_fallbacks++;
    test_fallback_exact = title && body && strcmp(title, "Build") == 0 && strcmp(body, "done ✓") == 0;
}
bool litty_notification_test_authorization_fallback(void) {
    LittyNotificationFallback saved = fallback;
    fallback = count_test_fallback;
    test_fallbacks = 0;
    test_fallback_exact = NO;
    NSError *failure = [NSError errorWithDomain:UNErrorDomain code:1 userInfo:nil];
    LittyNotificationTicket *granted = [LittyNotificationTicket new];
    BOOL granted_continues = !handle_authorization_failure(granted, YES, nil, @"Build", @"done ✓") && test_fallbacks == 0;
    LittyNotificationTicket *denied = [LittyNotificationTicket new];
    BOOL denied_handled = handle_authorization_failure(denied, NO, nil, @"Build", @"done ✓") && test_fallbacks == 1 && test_fallback_exact;
    // A completion already finished (for example by shutdown) must not repeat it.
    BOOL no_duplicate = handle_authorization_failure(denied, NO, nil, @"Build", @"done ✓") && test_fallbacks == 1;
    LittyNotificationTicket *errored = [LittyNotificationTicket new];
    BOOL error_handled = handle_authorization_failure(errored, YES, failure, @"Build", @"done ✓") && test_fallbacks == 2;
    fallback = saved;
    return granted_continues && denied_handled && no_duplicate && error_handled;
}

bool litty_notification_test_late_authorization(void) {
    LittyNotificationTicket *ticket = [LittyNotificationTicket new];
    // A simulated arbitrarily late authorization must retain the first request.
    BOOL expired_before_grant = expire_submission(ticket);
    BOOL retained = !expired_before_grant && !atomic_load(&ticket->finished);
    atomic_store(&ticket->submission_started, true);
    BOOL submission_deadline = expire_submission(ticket);
    return retained && submission_deadline && !expire_submission(ticket);
}

static NSString *track_identifier(NSMutableOrderedSet<NSString *> *identifiers, NSString *identifier) {
    [identifiers addObject:identifier];
    if (identifiers.count <= 64) return nil;
    NSString *oldest = identifiers.firstObject;
    [identifiers removeObjectAtIndex:0];
    return oldest;
}

static NSArray<NSString *> *take_owned_identifiers(NSMutableOrderedSet<NSString *> *identifiers) {
    NSArray<NSString *> *result = [identifiers.array copy];
    [identifiers removeAllObjects];
    return result;
}
bool litty_notification_test_selective_shutdown(void) {
    NSMutableOrderedSet<NSString *> *owned = [NSMutableOrderedSet orderedSetWithArray:@[@"own-1", @"own-2"]];
    NSArray *other = @[@"another-process-1"];
    NSArray *withdrawn = take_owned_identifiers(owned);
    BOOL selective = [withdrawn isEqualToArray:@[@"own-1", @"own-2"]] && owned.count == 0
        && [other isEqualToArray:@[@"another-process-1"]];
    NSString *evicted = nil;
    for (NSUInteger i = 0; i < 65; i++) evicted = track_identifier(owned, [NSString stringWithFormat:@"own-%lu", (unsigned long)i]);
    return selective && owned.count == 64 && [evicted isEqual:@"own-0"]
        && take_owned_identifiers(owned).count == 64 && owned.count == 0;
}

int litty_notification_show(const char *origin_bytes, uint64_t pane_id, const char *title_bytes, const char *body_bytes) {
    if (!available || atomic_load(&shutting_down)) return 1;
    if (atomic_fetch_add(&pending, 1) >= 8) { atomic_fetch_sub(&pending, 1); return 2; }
    NSString *origin = [NSString stringWithUTF8String:origin_bytes];
    NSString *title = [NSString stringWithUTF8String:title_bytes];
    NSString *body = [NSString stringWithUTF8String:body_bytes];
    if (!valid_origin(origin) || !title || !body || pane_id == 0) { atomic_fetch_sub(&pending, 1); return 2; }
    LittyNotificationTicket *ticket = [LittyNotificationTicket new];
    dispatch_async(dispatch_get_main_queue(), ^{
        UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
        if (atomic_load(&shutting_down)) { atomic_fetch_sub(&pending, 1); return; }
        [center requestAuthorizationWithOptions:UNAuthorizationOptionAlert completionHandler:^(BOOL granted, NSError *error) {
            dispatch_async(dispatch_get_main_queue(), ^{
                if (atomic_load(&shutting_down) || atomic_load(&ticket->finished)) { atomic_fetch_sub(&pending, 1); return; }
                if (handle_authorization_failure(ticket, granted, error, title, body)) {
                    atomic_fetch_sub(&pending, 1);
                    return;
                }
                // Human authorization has no transport deadline. Admission stays
                // bounded while the person decides; only native submission is timed.
                atomic_store(&ticket->submission_started, true);
                dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{
                    if (expire_submission(ticket)) fprintf(stderr, "litty: notification delivery timed out\n");
                });
                UNMutableNotificationContent *content = [UNMutableNotificationContent new];
                content.title = title;
                content.body = body;
                content.userInfo = @{@"origin": origin, @"pane": [NSString stringWithFormat:@"%llu", (unsigned long long)pane_id]};
                NSString *identifier = [NSString stringWithFormat:@"litty-pane-%@-%llu", origin, (unsigned long long)++sequence];
                NSString *oldest = track_identifier(owned_identifiers, identifier);
                if (oldest) {
                    [center removePendingNotificationRequestsWithIdentifiers:@[oldest]];
                    [center removeDeliveredNotificationsWithIdentifiers:@[oldest]];
                }
                UNNotificationRequest *request = [UNNotificationRequest requestWithIdentifier:identifier content:content trigger:nil];
                [center addNotificationRequest:request withCompletionHandler:^(NSError *delivery_error) {
                    dispatch_async(dispatch_get_main_queue(), ^{
                        if (finish(ticket) && delivery_error) fprintf(stderr, "litty: notification submission failed\n");
                        if (atomic_load(&shutting_down) || ![owned_identifiers containsObject:identifier]) {
                            // Completion can follow clean shutdown or retention
                            // eviction. Withdraw again to close that async race.
                            UNUserNotificationCenter *current = UNUserNotificationCenter.currentNotificationCenter;
                            [current removePendingNotificationRequestsWithIdentifiers:@[identifier]];
                            [current removeDeliveredNotificationsWithIdentifiers:@[identifier]];
                        }
                        atomic_fetch_sub(&pending, 1);
                    });
                }];
            });
        }];
    });
    return 0;
}

// Exercises the same parsing, deduplication and completion path as the OS
// callback without depending on desktop authorization or shared app state.
bool litty_notification_test_callbacks(void) {
    @autoreleasepool {
        LittyNotificationDelegate *delegate = [LittyNotificationDelegate new];
        delegate.seen = [NSMutableOrderedSet new];
        __block NSUInteger routed = 0;
        __block NSUInteger completed = 0;
        __block BOOL exact = NO;
        NSString *origin = @"0000000000000001-1-00000000000000000000000000000001";
        delegate.routeAction = ^(NSString *target, uint64_t pane) { routed++; exact = [target isEqual:origin] && pane == 42; };
        void (^done)(void) = ^{ completed++; };
        [delegate handleIdentifier:@"litty-pane-1" action:UNNotificationDefaultActionIdentifier origin:origin pane:@"42" completion:done];
        [delegate handleIdentifier:@"litty-pane-1" action:UNNotificationDefaultActionIdentifier origin:origin pane:@"42" completion:done];
        [delegate handleIdentifier:@"litty-pane-2" action:UNNotificationDismissActionIdentifier origin:origin pane:@"42" completion:done];
        [delegate handleIdentifier:@"litty-pane-3" action:UNNotificationDefaultActionIdentifier origin:@"../bad" pane:@"42" completion:done];
        [delegate handleIdentifier:@"litty-pane-4" action:UNNotificationDefaultActionIdentifier origin:origin pane:@"18446744073709551616" completion:done];
        [delegate handleIdentifier:@"litty-pane-5" action:UNNotificationDefaultActionIdentifier origin:origin pane:@"0" completion:done];
        delegate.closed = YES;
        [delegate handleIdentifier:@"litty-pane-after-close" action:UNNotificationDefaultActionIdentifier origin:origin pane:@"42" completion:done];
        return routed == 1 && completed == 7 && exact;
    }
}

void litty_notifications_shutdown(void) {
    if (!NSThread.isMainThread) return;
    atomic_store(&shutting_down, true);
    route = NULL;
    if (!available) return;
    NSArray<NSString *> *identifiers = take_owned_identifiers(owned_identifiers);
    UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
    [center removePendingNotificationRequestsWithIdentifiers:identifiers];
    [center removeDeliveredNotificationsWithIdentifiers:identifiers];
    notification_delegate.closed = YES;
    if (center.delegate == notification_delegate) center.delegate = nil;
    if (launch_observer) [NSNotificationCenter.defaultCenter removeObserver:launch_observer];
    launch_observer = nil;
    notification_delegate = nil;
    available = NO;
    // Already-enqueued blocks retain their objects. Their close checks stop
    // routing/submission and their completion handlers still run exactly once.
}

static BOOL notification_reopen_source(AEEventClass event_class, AEEventID event_id, const char *sender_path) {
    // Observed notification reopens originate from the system notification
    // daemon. Classify each event independently; never consume a later Dock click.
    return event_class == kCoreEventClass && event_id == kAEReopenApplication
        && sender_path && strcmp(sender_path, "/usr/sbin/usernoted") == 0;
}
bool litty_notification_test_reopen_provenance(void) {
    return notification_reopen_source(kCoreEventClass, kAEReopenApplication, "/usr/sbin/usernoted")
        && !notification_reopen_source(kCoreEventClass, kAEReopenApplication, "/System/Library/CoreServices/Dock.app/Contents/MacOS/Dock")
        && !notification_reopen_source(kCoreEventClass, kAEReopenApplication, "/tmp/usernoted")
        && !notification_reopen_source(kCoreEventClass, kAEReopenApplication, NULL)
        && !notification_reopen_source(kCoreEventClass, kAEOpenApplication, "/usr/sbin/usernoted");
}
