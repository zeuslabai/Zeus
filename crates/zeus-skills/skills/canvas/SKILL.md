# canvas

Canvas LMS integration for course management.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Canvas LMS assistant. Help users manage courses, assignments, grades, and announcements in their Canvas learning management system.

## Tools

### canvas_list_courses
List enrolled courses.
```json
{
  "type": "object",
  "properties": {
    "enrollment_state": {
      "type": "string",
      "enum": ["active", "completed", "all"],
      "default": "active"
    }
  }
}
```

### canvas_get_course
Get course details.
```json
{
  "type": "object",
  "properties": {
    "course_id": {
      "type": "string"
    }
  },
  "required": ["course_id"]
}
```

### canvas_list_assignments
List assignments for a course.
```json
{
  "type": "object",
  "properties": {
    "course_id": {
      "type": "string"
    },
    "bucket": {
      "type": "string",
      "enum": ["past", "overdue", "undated", "ungraded", "unsubmitted", "upcoming", "future"],
      "description": "Filter by status"
    }
  },
  "required": ["course_id"]
}
```

### canvas_get_assignment
Get assignment details.
```json
{
  "type": "object",
  "properties": {
    "course_id": {
      "type": "string"
    },
    "assignment_id": {
      "type": "string"
    }
  },
  "required": ["course_id", "assignment_id"]
}
```

### canvas_submit_assignment
Submit an assignment.
```json
{
  "type": "object",
  "properties": {
    "course_id": {
      "type": "string"
    },
    "assignment_id": {
      "type": "string"
    },
    "submission_type": {
      "type": "string",
      "enum": ["online_text_entry", "online_url", "online_upload"]
    },
    "body": {
      "type": "string",
      "description": "Text content or URL"
    },
    "file": {
      "type": "string",
      "description": "File path for upload"
    }
  },
  "required": ["course_id", "assignment_id", "submission_type"]
}
```

### canvas_list_grades
Get grades for a course.
```json
{
  "type": "object",
  "properties": {
    "course_id": {
      "type": "string"
    }
  },
  "required": ["course_id"]
}
```

### canvas_announcements
Get course announcements.
```json
{
  "type": "object",
  "properties": {
    "course_id": {
      "type": "string"
    },
    "limit": {
      "type": "integer",
      "default": 10
    }
  },
  "required": ["course_id"]
}
```

### canvas_calendar
Get calendar events.
```json
{
  "type": "object",
  "properties": {
    "start_date": {
      "type": "string"
    },
    "end_date": {
      "type": "string"
    },
    "course_id": {
      "type": "string",
      "description": "Filter by course"
    }
  }
}
```

## Commands

### list_courses
```bash
curl -s -H "Authorization: Bearer $CANVAS_API_KEY" \
  "$CANVAS_URL/api/v1/courses?enrollment_state={enrollment_state}"
```

### get_course
```bash
curl -s -H "Authorization: Bearer $CANVAS_API_KEY" \
  "$CANVAS_URL/api/v1/courses/{course_id}"
```

### list_assignments
```bash
curl -s -H "Authorization: Bearer $CANVAS_API_KEY" \
  "$CANVAS_URL/api/v1/courses/{course_id}/assignments"
```

### get_grades
```bash
curl -s -H "Authorization: Bearer $CANVAS_API_KEY" \
  "$CANVAS_URL/api/v1/courses/{course_id}/enrollments?user_id=self"
```

### announcements
```bash
curl -s -H "Authorization: Bearer $CANVAS_API_KEY" \
  "$CANVAS_URL/api/v1/announcements?context_codes[]=course_{course_id}&per_page={limit}"
```

## Environment
- CANVAS_API_KEY
- CANVAS_URL

## Permissions
- network
