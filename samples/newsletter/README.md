# Newsletter

Periodic newsletter subscription demonstrating time-based workflows and cancellation.

## Example usage via CLI

Spawn starts entity, enters loop, sleeps for 7 days:

    duralade entity spawn \
        --entity newsletter.subscription \
        --id sub1 \
        --in '{"email": "user@example.com"}'

Complete first sleep at +7d and tick - sends newsletter:

    duralade entity complete-extern \
        --id sub1 \
        --invoke-num <event_num> \
        --current-time +7d

    duralade entity tick --id sub1

Complete send and tick - increments counter, loops, sleeps again:

    duralade entity complete-extern \
        --id sub1 \
        --invoke-num <event_num>

    duralade entity tick --id sub1

Cancel and tick - gets canceled_error, sends farewell:

    duralade entity cancel --id sub1

    duralade entity tick --id sub1

Complete farewell and describe final status:

    duralade entity complete-extern \
        --id sub1 \
        --invoke-num <event_num>

    duralade entity describe --id sub1
