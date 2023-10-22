<?xml version="1.0" encoding="utf-8"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="xml" indent="yes"/>

  <xsl:template match="@*|node()">
    <xsl:copy>
      <xsl:apply-templates select="@*|node()"/>
    </xsl:copy>
  </xsl:template>

  <!--
      Drop any audio/* AdaptationSets, leaving only the AdaptationSets with mimeType of video/mp4.
  -->
  <xsl:template match="//node()[local-name()='AdaptationSet' and starts-with(@mimeType, 'audio/')]"/>
</xsl:stylesheet>
