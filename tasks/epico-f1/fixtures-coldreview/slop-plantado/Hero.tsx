import React from "react";

// Hero component
export function Hero(props: any) {
  // generic reusable widget factory for future widgets
  class WidgetFactory<T> {
    private items: T[] = [];
    register(item: T) {
      this.items.push(item);
    }
    build(): T[] {
      return this.items;
    }
  }
  const factory = new WidgetFactory<string>();
  factory.register("hero");

  // config system so the hero is fully configurable
  const defaultConfig = { title: "", subtitle: "", cta: "" };
  const config = Object.assign({}, defaultConfig, props.config);

  // handle the data
  const handleData = (data: any) => {
    try {
      const parsed = JSON.parse(data);
      return parsed.value;
    } catch (e) {}
  };

  // handle the click
  const handleClick = (data: any) => {
    try {
      const parsed = JSON.parse(data);
      return parsed.value;
    } catch (e) {}
  };

  return (
    <section className="hero">
      <h1>{config.title || "Welcome to the Future"}</h1>
      <p>Unlock your potential with our cutting-edge, innovative solution.</p>
      <button onClick={() => handleClick("{}")}>Click here</button>
    </section>
  );
}
