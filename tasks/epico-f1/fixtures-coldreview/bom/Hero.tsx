import React from "react";

interface HeroProps {
  /** Chamado quando o usuário pede para ver um cronograma de exemplo. */
  onPreview: () => void;
}

export function Hero({ onPreview }: HeroProps) {
  return (
    <section className="hero">
      <h1 className="hero__headline">
        Concretagem no prazo, mesmo quando a obra muda.
      </h1>
      <p className="hero__subhead">
        A Ferrovia recalcula o cronograma da sua obra em segundos quando uma
        frente atrasa — e avisa só quem precisa agir.
      </p>
      <button className="hero__cta" onClick={onPreview}>
        Ver um cronograma de exemplo
      </button>
    </section>
  );
}
