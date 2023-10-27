<?xml version="1.0" encoding="utf-8"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="xml" indent="yes"/>

  <xsl:template match="@*|node()">
    <xsl:copy>
      <xsl:apply-templates select="@*|node()"/>
    </xsl:copy>
  </xsl:template>
  
  <!--
      This stylesheet modifies the @initialization and @media attribute on SegmentTemplate elements,
      as well as the content of BaseURL elements, to point to a beloved media segment. It also drops
      the audio AdaptationSet.

      To test this stylesheet:

      xsltproc rewrite-rickroll.xslt input.mpd
  -->

  <xsl:template match="//node()[local-name()='BaseURL']">
    <BaseURL>https://dash.akamaized.net/akamai/test/rick_dash_track1_init.mp4</BaseURL>
  </xsl:template>
  
  <xsl:template match="//node()[local-name()='SegmentTemplate']/@initialization">
    <xsl:attribute name="initialization">
      <xsl:value-of select="'https://dash.akamaized.net/akamai/test/rick_dash_track1_init.mp4'"/>
    </xsl:attribute>
  </xsl:template>

  <xsl:template match="//node()[local-name()='SegmentTemplate']/@media">
    <xsl:attribute name="media">
      <xsl:value-of select="'https://dash.akamaized.net/akamai/test/rick_dash_track1_init.mp4'"/>
    </xsl:attribute>
  </xsl:template>

  <xsl:template match="//node()[local-name()='AdaptationSet' and @contentType='audio']"/>
  <xsl:template match="//node()[local-name()='AdaptationSet' and starts-with(@mimeType, 'audio/')]"/>
</xsl:stylesheet>
