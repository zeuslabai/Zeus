# food-order

Interface with food delivery services (DoorDash, UberEats) for menu browsing.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a food ordering assistant. Help users browse restaurant menus, find deals, and explore food delivery options. Note: actual ordering requires user authentication on the respective platforms.

## Tools

### food_search_restaurants
Search for restaurants that deliver.
```json
{
  "type": "object",
  "properties": {
    "location": {
      "type": "string",
      "description": "Delivery address or zip code"
    },
    "cuisine": {
      "type": "string",
      "description": "Cuisine type filter"
    },
    "sort_by": {
      "type": "string",
      "enum": ["rating", "delivery_time", "distance", "popularity"],
      "default": "popularity"
    }
  },
  "required": ["location"]
}
```

### food_restaurant_menu
Get menu for a restaurant.
```json
{
  "type": "object",
  "properties": {
    "restaurant_id": {
      "type": "string"
    },
    "category": {
      "type": "string",
      "description": "Menu category filter"
    }
  },
  "required": ["restaurant_id"]
}
```

### food_search_dishes
Search for specific dishes.
```json
{
  "type": "object",
  "properties": {
    "dish": {
      "type": "string",
      "description": "Dish name (e.g., 'pad thai', 'burger')"
    },
    "location": {
      "type": "string"
    }
  },
  "required": ["dish", "location"]
}
```

### food_deals
Find current deals and promotions.
```json
{
  "type": "object",
  "properties": {
    "location": {
      "type": "string"
    }
  },
  "required": ["location"]
}
```

### food_restaurant_info
Get restaurant details (hours, ratings, etc.).
```json
{
  "type": "object",
  "properties": {
    "restaurant_id": {
      "type": "string"
    }
  },
  "required": ["restaurant_id"]
}
```

### food_estimate_delivery
Estimate delivery time and fee.
```json
{
  "type": "object",
  "properties": {
    "restaurant_id": {
      "type": "string"
    },
    "address": {
      "type": "string"
    }
  },
  "required": ["restaurant_id", "address"]
}
```

## Commands

### search_yelp
```bash
curl -s "https://api.yelp.com/v3/businesses/search?location={location}&categories=restaurants&term={cuisine}" \
  -H "Authorization: Bearer $YELP_API_KEY" | jq '.businesses[] | {name, rating, price, location: .location.address1}'
```

### restaurant_details
```bash
curl -s "https://api.yelp.com/v3/businesses/{restaurant_id}" \
  -H "Authorization: Bearer $YELP_API_KEY"
```

## Environment
- YELP_API_KEY

## Permissions
- network
