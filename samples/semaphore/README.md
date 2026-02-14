# Semaphore

Counting semaphore with blocking acquire, non-blocking try-acquire, and release.

## Example usage via CLI

Create a semaphore with 3 permits:

    duralade entity spawn \
        --entity semaphore.semaphore \
        --id sem1 \
        --in '{"permits": 3}'

Try to acquire a permit (non-blocking):

    duralade entity invoke-func-noblock \
        --id sem1 \
        --func acquire_try

Check remaining permits:

    duralade entity view \
        --id sem1 \
        --field permits

Release a permit:

    duralade entity invoke-func-noblock \
        --id sem1 \
        --func release
