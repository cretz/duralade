# Shopping Cart

Stateful workflows with prepare/apply pattern for checkout.

## Example usage via CLI

Create an empty shopping cart:

    duralade entity spawn \
        --entity shopping_cart.cart \
        --id cart1

Add items to cart:

    duralade entity invoke-func \
        --id cart1 \
        --func item_add \
        --in '{"product_id": "prod1", "quantity": 2}'

    duralade entity tick --id cart1

Complete product_lookup extern:

    duralade entity complete-extern \
        --id cart1 \
        --invoke-num <event_num> \
        --result '{"name": "Widget", "price": 1000}'

    duralade entity tick --id cart1

Get current cart items:

    duralade entity view \
        --id cart1 \
        --func items_get

Prepare checkout (calculates subtotal and tax):

    duralade entity invoke-func \
        --id cart1 \
        --func checkout_prepare

    duralade entity tick --id cart1

Complete checkout_prepare_apply extern:

    duralade entity complete-extern \
        --id cart1 \
        --invoke-num <event_num> \
        --result '{"subtotal": 2000, "tax": 200}'

    duralade entity tick --id cart1

Apply checkout (completes the order):

    duralade entity invoke-func \
        --id cart1 \
        --func checkout

    duralade entity tick --id cart1

Complete checkout_apply extern:

    duralade entity complete-extern \
        --id cart1 \
        --invoke-num <event_num> \
        --result '{"order_id": "order-123"}'

    duralade entity tick --id cart1

Check final status and order_id:

    duralade entity describe --id cart1
