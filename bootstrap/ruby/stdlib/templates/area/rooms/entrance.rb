class Entrance < Room
  title "The Entrance"
  description "Welcome to {{area_name}}."
  exit :north, to: "hall"
end
